use serde::{Deserialize, Serialize};

/// A single summary statistic.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Statistic {
    Count,
    PercentOfParent,
    PercentOfTotal,
    Mean,
    Median,
    StdDev,
    Cv, // coefficient of variation (std / mean)
    GeoMean, // geometric mean (of positive values)
    Min,
    Max,
}

/// Computed statistics for one population across one parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PopulationStat {
    pub parameter: String,
    pub statistic: Statistic,
    pub value: f64,
}

/// A full statistical profile of a population: one `PopulationStat` per
/// (parameter, statistic) pair.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PopulationProfile {
    pub name: String,
    pub count: usize,
    pub parent_count: Option<usize>,
    pub total_count: Option<usize>,
    pub stats: Vec<PopulationStat>,
}

impl PopulationProfile {
    pub fn percent_of_parent(&self) -> Option<f64> {
        self.parent_count.map(|pc| {
            if pc == 0 {
                0.0
            } else {
                (self.count as f64 / pc as f64) * 100.0
            }
        })
    }

    pub fn percent_of_total(&self) -> Option<f64> {
        self.total_count.map(|tc| {
            if tc == 0 {
                0.0
            } else {
                (self.count as f64 / tc as f64) * 100.0
            }
        })
    }
}

/// Compute statistics for a subset of events (a population) on a dataset.
///
/// `data` has shape `(events, parameters)`. `indices` selects the events
/// belonging to the population. `parameter_names` labels each column.
pub fn compute_profile(
    name: &str,
    data: &ndarray::Array2<f32>,
    indices: &[usize],
    parameter_names: &[String],
    parent_count: Option<usize>,
    total_count: Option<usize>,
) -> PopulationProfile {
    let (_, n_params) = data.dim();
    let mut stats: Vec<PopulationStat> = Vec::new();

    // Overall count stats are pushed first.
    let count = indices.len();
    let profile_count = count;
    let pct_parent = parent_count.map(|pc| {
        if pc == 0 {
            0.0
        } else {
            (count as f64 / pc as f64) * 100.0
        }
    });
    let pct_total = total_count.map(|tc| {
        if tc == 0 {
            0.0
        } else {
            (count as f64 / tc as f64) * 100.0
        }
    });

    // Percent stats as pseudo-parameters named "%Parent"/"%Total".
    if let Some(v) = pct_parent {
        stats.push(PopulationStat {
            parameter: "%Parent".to_string(),
            statistic: Statistic::PercentOfParent,
            value: v,
        });
    }
    if let Some(v) = pct_total {
        stats.push(PopulationStat {
            parameter: "%Total".to_string(),
            statistic: Statistic::PercentOfTotal,
            value: v,
        });
    }

    for p in 0..n_params {
        let name = parameter_names.get(p).cloned().unwrap_or_else(|| format!("P{p}"));
        // Collect the population's values for this parameter.
        let mut values: Vec<f64> = indices.iter().map(|&i| data[[i, p]] as f64).collect();
        if values.is_empty() {
            continue;
        }

        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let sum: f64 = values.iter().sum();
        let mean = sum / values.len() as f64;
        let variance: f64 = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
            / values.len() as f64;
        let std_dev = variance.sqrt();
        let cv = if mean != 0.0 { std_dev / mean.abs() } else { f64::NAN };

        let median = if values.len() % 2 == 1 {
            values[values.len() / 2]
        } else {
            let mid = values.len() / 2;
            (values[mid - 1] + values[mid]) / 2.0
        };

        // Geometric mean of strictly positive values.
        let geo_mean: f64 = if values.iter().all(|v| *v > 0.0) {
            values.iter().map(|v| v.ln()).sum::<f64>() / values.len() as f64
        } else {
            f64::NAN
        }
        .exp();

        let min = values[0];
        let max = values[values.len() - 1];

        stats.push(PopulationStat {
            parameter: name.clone(),
            statistic: Statistic::Count,
            value: count as f64,
        });
        stats.push(PopulationStat {
            parameter: name.clone(),
            statistic: Statistic::Mean,
            value: mean,
        });
        stats.push(PopulationStat {
            parameter: name.clone(),
            statistic: Statistic::Median,
            value: median,
        });
        stats.push(PopulationStat {
            parameter: name.clone(),
            statistic: Statistic::StdDev,
            value: std_dev,
        });
        stats.push(PopulationStat {
            parameter: name.clone(),
            statistic: Statistic::Cv,
            value: cv,
        });
        stats.push(PopulationStat {
            parameter: name.clone(),
            statistic: Statistic::GeoMean,
            value: geo_mean,
        });
        stats.push(PopulationStat {
            parameter: name.clone(),
            statistic: Statistic::Min,
            value: min,
        });
        stats.push(PopulationStat {
            parameter: name,
            statistic: Statistic::Max,
            value: max,
        });
    }

    PopulationProfile {
        name: name.to_string(),
        count: profile_count,
        parent_count,
        total_count,
        stats,
    }
}

/// Pretty-print a population profile to stdout.
pub fn print_profile(profile: &PopulationProfile) {
    println!("Population: {}", profile.name);
    println!("  count = {}", profile.count);
    if let Some(p) = profile.percent_of_parent() {
        println!("  % of parent = {:.2}%", p);
    }
    if let Some(p) = profile.percent_of_total() {
        println!("  % of total = {:.2}%", p);
    }
    for stat in &profile.stats {
        match stat.statistic {
            Statistic::Count | Statistic::PercentOfParent | Statistic::PercentOfTotal => {}
            _ => {
                let label = match stat.statistic {
                    Statistic::Mean => "mean",
                    Statistic::Median => "median",
                    Statistic::StdDev => "std",
                    Statistic::Cv => "CV",
                    Statistic::GeoMean => "geoMean",
                    Statistic::Min => "min",
                    Statistic::Max => "max",
                    _ => "?",
                };
                if stat.value.is_nan() {
                    println!("  {} {} = NaN", stat.parameter, label);
                } else {
                    println!("  {} {} = {:.4}", stat.parameter, label, stat.value);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    #[test]
    fn profile_basic_stats() {
        let data = array![[1.0], [2.0], [3.0], [4.0], [5.0]];
        let indices = vec![0, 1, 2, 3, 4];
        let profile = compute_profile(
            "All",
            &data,
            &indices,
            &["FSC".to_string()],
            None,
            Some(5),
        );
        assert_eq!(profile.count, 5);
        let mean = profile
            .stats
            .iter()
            .find(|s| s.statistic == Statistic::Mean && s.parameter == "FSC")
            .unwrap()
            .value;
        assert!((mean - 3.0).abs() < 1e-6);
        let median = profile
            .stats
            .iter()
            .find(|s| s.statistic == Statistic::Median && s.parameter == "FSC")
            .unwrap()
            .value;
        assert!((median - 3.0).abs() < 1e-6);
        let pct = profile.percent_of_total().unwrap();
        assert!((pct - 100.0).abs() < 1e-6);
    }

    #[test]
    fn subset_percent() {
        let data = array![[1.0], [2.0], [10.0], [20.0]];
        let indices = vec![2, 3];
        let profile = compute_profile(
            "High",
            &data,
            &indices,
            &["FSC".to_string()],
            Some(4),
            Some(4),
        );
        assert_eq!(profile.count, 2);
        assert!((profile.percent_of_parent().unwrap() - 50.0).abs() < 1e-6);
        assert!((profile.percent_of_total().unwrap() - 50.0).abs() < 1e-6);
    }
}
