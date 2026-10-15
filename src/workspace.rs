use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;

use crate::fcs::FcsFile;
use crate::gating::GatedPopulation;
use crate::statistics::{compute_profile, PopulationProfile};

/// A node in the hierarchical population tree.
///
/// Each population holds the event indices (into a sample's data matrix) that
/// pass its gate, plus any child sub-populations.
#[derive(Debug, Clone)]
pub struct Population {
    pub name: String,
    pub event_indices: Vec<usize>,
    pub children: Vec<Rc<RefCell<Population>>>,
    pub gate_summary: Option<String>,
}

impl Population {
    pub fn new(name: &str, indices: Vec<usize>) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Population {
            name: name.to_string(),
            event_indices: indices,
            children: Vec::new(),
            gate_summary: None,
        }))
    }

    pub fn add_child(&mut self, child: Rc<RefCell<Self>>) {
        self.children.push(child);
    }

    pub fn count(&self) -> usize {
        self.event_indices.len()
    }

    pub fn set_gate_summary(&mut self, summary: &str) {
        self.gate_summary = Some(summary.to_string());
    }
}

/// A serializable snapshot of a population (for project files).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PopulationSnapshot {
    pub name: String,
    pub count: usize,
    pub gate_summary: Option<String>,
    pub children: Vec<PopulationSnapshot>,
}

impl Population {
    /// Flatten this population and its descendants into snapshots.
    pub fn to_snapshot(root: &Rc<RefCell<Self>>) -> PopulationSnapshot {
        let p = root.borrow();
        PopulationSnapshot {
            name: p.name.clone(),
            count: p.event_indices.len(),
            gate_summary: p.gate_summary.clone(),
            children: p
                .children
                .iter()
                .map(|c| Population::to_snapshot(c))
                .collect(),
        }
    }
}

/// The top-level experiment workspace: samples + population trees + stats.
///
/// Note: `Workspace` and `Sample` are not `Serialize` because they hold
/// `Rc<RefCell<Population>>` and `FcsFile` (the latter lacks a serde impl).
/// Persist a project by serializing `PopulationSnapshot`s and FCS metadata
/// separately.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub samples: HashMap<String, Sample>,
    /// Ordered list of sample IDs for stable display order.
    pub sample_order: Vec<String>,
    /// Computed profiles keyed by (sample_id, population_path).
    pub profiles: HashMap<String, PopulationProfile>,
}

#[derive(Debug, Clone)]
pub struct Sample {
    pub id: String,
    pub fcs_file: FcsFile,
    /// Root populations for this sample.
    pub root_populations: Vec<Rc<RefCell<Population>>>,
    /// Gated populations produced by the user (flat list with paths).
    pub gates: Vec<GatedPopulation>,
}

impl Sample {
    pub fn new(id: &str, fcs_file: FcsFile) -> Self {
        Self {
            id: id.to_string(),
            fcs_file,
            root_populations: Vec::new(),
            gates: Vec::new(),
        }
    }
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            samples: HashMap::new(),
            sample_order: Vec::new(),
            profiles: HashMap::new(),
        }
    }

    pub fn add_sample(&mut self, id: &str, fcs_file: FcsFile) {
        let sample = Sample::new(id, fcs_file);
        self.samples.insert(id.to_string(), sample);
        self.sample_order.push(id.to_string());
    }

    pub fn get_sample(&self, id: &str) -> Option<&Sample> {
        self.samples.get(id)
    }

    pub fn get_sample_mut(&mut self, id: &str) -> Option<&mut Sample> {
        self.samples.get_mut(id)
    }

    /// Add a root (top-level) population to a sample.
    pub fn add_root_population(
        &mut self,
        sample_id: &str,
        name: &str,
        indices: Vec<usize>,
    ) -> Option<Rc<RefCell<Population>>> {
        let sample = self.get_sample_mut(sample_id)?;
        let pop = Population::new(name, indices);
        sample.root_populations.push(Rc::clone(&pop));
        Some(pop)
    }

    /// Compute and cache the statistical profile for a gated population.
    pub fn compute_profile_for(
        &mut self,
        sample_id: &str,
        population_path: &str,
        indices: &[usize],
    ) -> PopulationProfile {
        let sample = self.samples.get(sample_id).expect("sample must exist");
        let fcs = &sample.fcs_file;
        let param_names: Vec<String> = fcs
            .parameters
            .iter()
            .map(|p| p.name.clone())
            .collect();
        let total_count = Some(fcs.event_count);
        let profile = compute_profile(
            population_path,
            &fcs.data,
            indices,
            &param_names,
            None,
            total_count,
        );
        let key = format!("{sample_id}::{population_path}");
        self.profiles.insert(key.clone(), profile.clone());
        profile
    }

    /// List all cached profiles.
    pub fn profiles(&self) -> impl Iterator<Item = (&String, &PopulationProfile)> {
        self.profiles.iter()
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fcs::FCS_HEADER_LEN;

    #[test]
    fn add_and_compute_profile() {
        let mut ws = Workspace::new();
        let fcs = crate::fcs::FcsParser::parse_bytes(&build_fcs(4, 2)).unwrap();
        ws.add_sample("s1", fcs);

        let indices = vec![0, 1];
        ws.add_root_population("s1", "Live", indices.clone());
        let profile = ws.compute_profile_for("s1", "Live", &indices);
        assert_eq!(profile.count, 2);
        assert!(ws.profiles().count() >= 1);
    }

    fn build_fcs(events: usize, params: usize) -> Vec<u8> {
        // Minimal valid FCS 3.0 with proper header.
        // Pointers are 0-indexed byte offsets, matching real-world FCS files.
        let mut text = Vec::new();
        text.push(b'/');
        text.extend_from_slice(format!("$TOT/{events}/").as_bytes());
        text.extend_from_slice(format!("$PAR/{params}/").as_bytes());
        for n in 1..=params {
            text.extend_from_slice(format!("$P{n}N/P{n}/").as_bytes());
            text.extend_from_slice(format!("$P{n}R/4.0/").as_bytes());
        }
        text.extend_from_slice(format!("$BYTEORD/1,2,3,4/").as_bytes());

        let text_start = FCS_HEADER_LEN; // 0-indexed, right after header
        let text_end = text_start + text.len();
        let data_start = text_end;
        let data_end = data_start;
        let analysis_start = data_end + 1;
        let analysis_end = analysis_start + 1;
        fn ptr(n: usize) -> [u8; 8] {
            let s = format!("{n:>8}");
            let mut b = [0u8; 8];
            b.copy_from_slice(s.as_bytes());
            b
        }

        let mut buf = Vec::new();
        buf.extend_from_slice(b"FCS3.0    "); // version (6) + reserved (4) = 10 bytes
        buf.extend_from_slice(&ptr(text_start));
        buf.extend_from_slice(&ptr(text_end));
        buf.extend_from_slice(&ptr(data_start));
        buf.extend_from_slice(&ptr(data_end));
        buf.extend_from_slice(&ptr(analysis_start));
        buf.extend_from_slice(&ptr(analysis_end));
        buf.extend_from_slice(&text);
        for i in 0..events {
            for _ in 0..params {
                buf.extend_from_slice(&(i as f32).to_le_bytes());
            }
        }
        buf
    }
}
