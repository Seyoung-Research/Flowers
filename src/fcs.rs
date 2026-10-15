use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::num::ParseIntError;
use thiserror::Error;

use byteorder::{BigEndian, LittleEndian, ReadBytesExt};
use ndarray::Array2;

/// Errors produced while parsing FCS files.
#[derive(Debug, Error)]
pub enum FcsError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("UTF-8 error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("Number parse error: {0}")]
    ParseInt(#[from] ParseIntError),
    #[error("FCS parsing error: {0}")]
    Parse(String),
    #[error("Invalid FCS version: {0}")]
    InvalidVersion(String),
    #[error("Unsupported byte order: {0}")]
    UnsupportedByteOrder(String),
    #[error("Unsupported data type: {0}")]
    UnsupportedDataType(String),
    #[error("Missing required keyword: {0}")]
    MissingKeyword(String),
}

/// A single FCS parameter (channel) definition.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Parameter {
    pub name: String,        // $PnN  e.g. "FSC-H"
    pub description: String, // $PnS  e.g. "Forward Scatter-Height"
    pub range: f32,          // $PnR  measurement range
    pub exponent: f32,       // $PnE  exponent (e.g. 0 for linear)
    pub min: f32,            // computed display minimum
    pub max: f32,            // computed display maximum
}

/// A parsed FCS file.
///
/// `data` is intentionally not `Serialize` because `ndarray::Array2` lacks a
/// serde implementation; serialize project data via `PopulationSnapshot` instead.
#[derive(Debug, Clone, PartialEq)]
pub struct FcsFile {
    pub version: String,
    pub keywords: HashMap<String, String>,
    pub parameters: Vec<Parameter>,
    pub event_count: usize,
    pub data: Array2<f32>, // shape: (events, parameters)
}

/// Entry point for parsing FCS files.
///
/// Supports FCS 2.0, 3.0, and 3.1.  FCS 3.1 files may use either 32-bit
/// floats (`$DATATYPE/F`) or 64-bit doubles (`$DATATYPE/D`) for the data
/// segment; both are read and converted to `f32`.
pub struct FcsParser;

/// The FCS header is exactly 58 bytes long for FCS 2.0, 3.0, and 3.1.
pub(crate) const FCS_HEADER_LEN: usize = 58;

// Offsets within the 58-byte header (0-indexed). Each segment pointer is an
// 8-byte, space-padded, right-justified byte offset.  Real-world FCS 3.1
// files store these as **0-indexed** byte offsets into the file.
const HEADER_TEXT_START: (usize, usize) = (10, 18);
const HEADER_TEXT_END: (usize, usize) = (18, 26);
const HEADER_DATA_START: (usize, usize) = (26, 34);
const HEADER_DATA_END: (usize, usize) = (34, 42);
#[allow(dead_code)]
const HEADER_ANALYSIS_START: (usize, usize) = (42, 50);
#[allow(dead_code)]
const HEADER_ANALYSIS_END: (usize, usize) = (50, 58);

impl FcsParser {
    /// Parse an FCS 2.0, 3.0, or 3.1 file from any `Read + Seek` source.
    pub fn parse<R: Read + Seek>(mut reader: R) -> Result<FcsFile, FcsError> {
        // 1. Read and validate the fixed-size header.
        let mut header = [0u8; FCS_HEADER_LEN];
        reader.read_exact(&mut header)?;

        let version = std::str::from_utf8(&header[0..6])
            .map_err(|e| FcsError::Parse(format!("invalid header magic: {e}")))?
            .trim()
            .to_string();
        if !version.starts_with("FCS2")
            && !version.starts_with("FCS3.0")
            && !version.starts_with("FCS3.1")
        {
            return Err(FcsError::InvalidVersion(version));
        }

        // 2. Resolve segment byte ranges from the header pointers.
        // These are 0-indexed byte offsets into the file.
        let text_start = read_header_pointer(&header, HEADER_TEXT_START)? as u64;
        let text_end = read_header_pointer(&header, HEADER_TEXT_END)? as u64;
        let data_start = read_header_pointer(&header, HEADER_DATA_START)? as u64;
        let _data_end = read_header_pointer(&header, HEADER_DATA_END)? as u64;

        // 3. Parse the TEXT section into keyword/value pairs.
        reader.seek(SeekFrom::Start(text_start))?;
        let text_len = text_end.saturating_sub(text_start) as usize;
        let mut text_buf = vec![0u8; text_len];
        reader.read_exact(&mut text_buf)?;
        let text = std::str::from_utf8(&text_buf)?;
        let keywords = parse_text_section(text)?;

        // 4. Derive event count and parameter count.
        let event_count = keyword_usize(&keywords, "$TOT")?;
        let param_count = keyword_usize(&keywords, "$PAR")?;

        // 5. Read parameter definitions $P1N..$PnN, $P1S, $P1E, $P1R.
        let mut parameters = Vec::with_capacity(param_count);
        for n in 1..=param_count {
            let key_n = format!("$P{n}N");
            let key_s = format!("$P{n}S");
            let key_e = format!("$P{n}E");
            let key_r = format!("$P{n}R");

            let name = keyword_str(&keywords, &key_n)?
                .unwrap_or_else(|| format!("P{n}"));
            let description = keyword_str(&keywords, &key_s)?
                .unwrap_or_else(|| name.clone());
            let exponent: f32 = keyword_f32(&keywords, &key_e).unwrap_or(0.0);
            let range: f32 = keyword_f32(&keywords, &key_r).unwrap_or(4.0);

            parameters.push(Parameter {
                name,
                description,
                range,
                exponent,
                min: 0.0,
                max: range,
            });
        }

        // 6. Determine byte order ($BYTEORD: "1,2,3,4" = little-endian).
        let byte_order = keyword_str(&keywords, "$BYTEORD")?;
        let little_endian = match byte_order.as_deref() {
            None | Some("1,2,3,4") | Some("1,2,3,4 ") => true,
            Some("4,3,2,1") => false,
            Some(other) => return Err(FcsError::UnsupportedByteOrder(other.to_string())),
        };

        // 7. Determine data type ($DATATYPE: "F" = 32-bit float, "D" = 64-bit double).
        let data_type = keyword_str(&keywords, "$DATATYPE")?;
        let data_type = data_type.as_deref().unwrap_or("F");

        // 8. Read the DATA segment: event-major listmode values.
        reader.seek(SeekFrom::Start(data_start))?;
        let mut data = Array2::<f32>::zeros((event_count, param_count));

        for i in 0..event_count {
            for j in 0..param_count {
                let val: f32 = match data_type {
                    "F" => {
                        if little_endian {
                            reader.read_f32::<LittleEndian>()?
                        } else {
                            reader.read_f32::<BigEndian>()?
                        }
                    }
                    "D" => {
                        if little_endian {
                            reader.read_f64::<LittleEndian>()? as f32
                        } else {
                            reader.read_f64::<BigEndian>()? as f32
                        }
                    }
                    other => {
                        return Err(FcsError::UnsupportedDataType(other.to_string()));
                    }
                };
                data[[i, j]] = val;
            }
        }

        Ok(FcsFile {
            version,
            keywords,
            parameters,
            event_count,
            data,
        })
    }

    /// Parse an FCS file from a byte slice (convenience for tests / embedded data).
    pub fn parse_bytes(bytes: &[u8]) -> Result<FcsFile, FcsError> {
        let mut cursor = std::io::Cursor::new(bytes);
        Self::parse(&mut cursor)
    }
}

/// Read an 8-byte, space-padded, right-justified pointer from the header.
///
/// Returns the raw byte offset (0-indexed) as stored in the file.
fn read_header_pointer(header: &[u8; FCS_HEADER_LEN], (start, end): (usize, usize)) -> Result<usize, FcsError> {
    let field = std::str::from_utf8(&header[start..end])
        .map_err(|e| FcsError::Parse(format!("invalid header pointer: {e}")))?;
    let trimmed = field.trim();
    if trimmed.is_empty() {
        return Err(FcsError::Parse("empty header segment pointer".into()));
    }
    trimmed
        .parse::<usize>()
        .map_err(|e| FcsError::Parse(format!("invalid header pointer '{trimmed}': {e}")))
}

/// Parse the TEXT section: `delimiter KEYWORD delimiter VALUE delimiter ...`
///
/// The first character of the section is the delimiter (commonly `\` or `/`).
fn parse_text_section(text: &str) -> Result<HashMap<String, String>, FcsError> {
    let mut keywords = HashMap::new();
    let mut chars = text.chars();
    let delimiter = match chars.next() {
        Some(d) if !d.is_whitespace() => d,
        _ => return Ok(keywords), // empty or whitespace-only text section
    };

    // Split on the delimiter. Escape sequences (two delimiters in a row) are
    // collapsed into a single literal delimiter inside a value.
    let parts: Vec<&str> = text.split(delimiter).collect();
    let mut i = 1;
    while i + 1 < parts.len() {
        let key = parts[i].trim();
        let mut value = parts[i + 1].trim().to_string();
        // Unescape doubled delimiters.
        let doubled = format!("{delimiter}{delimiter}");
        if value.contains(&doubled) {
            value = value.replace(&doubled, &delimiter.to_string());
        }
        if !key.is_empty() {
            keywords.insert(key.to_uppercase(), value);
        }
        i += 2;
    }
    Ok(keywords)
}

fn keyword_str(keywords: &HashMap<String, String>, key: &str) -> Result<Option<String>, FcsError> {
    Ok(keywords.get(key).cloned())
}

fn keyword_usize(keywords: &HashMap<String, String>, key: &str) -> Result<usize, FcsError> {
    let value = keyword_str(keywords, key)?
        .ok_or_else(|| FcsError::MissingKeyword(key.to_string()))?;
    value
        .parse::<usize>()
        .map_err(|e| FcsError::Parse(format!("keyword '{key}' value '{value}' is not a usize: {e}")))
}

fn keyword_f32(keywords: &HashMap<String, String>, key: &str) -> Result<f32, FcsError> {
    let value = keyword_str(keywords, key)?
        .ok_or_else(|| FcsError::MissingKeyword(key.to_string()))?;
    value
        .parse::<f32>()
        .map_err(|e| FcsError::Parse(format!("keyword '{key}' value '{value}' is not an f32: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal but valid FCS file in memory with a proper header.
    ///
    /// `version` is the 6-byte version string (e.g. "FCS3.0" or "FCS3.1").
    /// Pointers are 0-indexed byte offsets, matching real-world FCS files.
    fn build_test_fcs(version: &str, event_count: usize, param_count: usize) -> Vec<u8> {
        // TEXT section first so we know its byte length for the header pointers.
        // Format: delimiter KEY delimiter VALUE delimiter ... (leading delimiter once).
        let mut text = Vec::new();
        text.push(b'/');
        text.extend_from_slice(format!("$TOT/{event_count}/").as_bytes());
        text.extend_from_slice(format!("$PAR/{param_count}/").as_bytes());
        for n in 1..=param_count {
            text.extend_from_slice(format!("$P{n}N/P{n}/").as_bytes());
            text.extend_from_slice(format!("$P{n}S/P{n}/").as_bytes());
            text.extend_from_slice(format!("$P{n}E/0/").as_bytes());
            text.extend_from_slice(format!("$P{n}R/4.0/").as_bytes());
        }
        text.extend_from_slice(format!("$BYTEORD/1,2,3,4/").as_bytes());
        text.extend_from_slice(format!("$DATATYPE/F/").as_bytes());
        text.extend_from_slice(format!("$MODE/L/").as_bytes());

        let text_start = FCS_HEADER_LEN; // 0-indexed, right after the 58-byte header
        let text_end = text_start + text.len();
        let data_start = text_end;

        fn ptr(n: usize) -> [u8; 8] {
            let s = format!("{n:>8}");
            let mut b = [0u8; 8];
            b.copy_from_slice(s.as_bytes());
            b
        }

        let mut buf = Vec::new();
        // Header: version (6 bytes) + 4 reserved + six 8-byte pointers.
        buf.extend_from_slice(version.as_bytes());
        // Pad version to 6 bytes (already exactly 6 for FCS3.0/FCS3.1).
        buf.extend_from_slice(&[b' '; 4]); // reserved bytes 6-9
        buf.extend_from_slice(&ptr(text_start));
        buf.extend_from_slice(&ptr(text_end));
        buf.extend_from_slice(&ptr(data_start));
        buf.extend_from_slice(&ptr(data_start)); // data end (unused)
        buf.extend_from_slice(&ptr(data_start + 1)); // analysis start (unused)
        buf.extend_from_slice(&ptr(data_start + 2)); // analysis end (unused)
        // TEXT section.
        buf.extend_from_slice(&text);
        // DATA section: event_count * param_count little-endian f32 values.
        for i in 0..event_count {
            for _j in 0..param_count {
                let val = (i as f32) * 0.1;
                buf.extend_from_slice(&val.to_le_bytes());
            }
        }
        buf
    }

    #[test]
    fn parses_fcs30_round_trip() {
        let fcs = build_test_fcs("FCS3.0", 10, 3);
        let parsed = FcsParser::parse_bytes(&fcs).unwrap();
        assert_eq!(parsed.version, "FCS3.0");
        assert_eq!(parsed.event_count, 10);
        assert_eq!(parsed.parameters.len(), 3);
        assert_eq!(parsed.data.dim(), (10, 3));
        assert!((parsed.data[[0, 0]] - 0.0).abs() < f32::EPSILON);
        assert!((parsed.data[[9, 2]] - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn parses_fcs31_round_trip() {
        let fcs = build_test_fcs("FCS3.1", 10, 3);
        let parsed = FcsParser::parse_bytes(&fcs).unwrap();
        assert_eq!(parsed.version, "FCS3.1");
        assert_eq!(parsed.event_count, 10);
        assert_eq!(parsed.parameters.len(), 3);
        assert_eq!(parsed.data.dim(), (10, 3));
    }

    #[test]
    fn rejects_bad_version() {
        let mut bad = build_test_fcs("FCS3.0", 5, 2);
        bad[0..6].copy_from_slice(b"FCS1.0");
        assert!(FcsParser::parse_bytes(&bad).is_err());
    }
}
