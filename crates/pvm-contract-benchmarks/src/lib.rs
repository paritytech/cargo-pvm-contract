//! Binary size comparison tool for PolkaVM contracts.

use std::path::Path;
use tabled::Tabled;
use walkdir::WalkDir;

#[derive(Debug, Clone, Tabled)]
pub struct ContractVariant {
    #[tabled(rename = "Contract")]
    pub name: String,
    #[tabled(rename = "Variant")]
    pub variant: String,
    #[tabled(rename = "Profile")]
    pub profile: String,
    #[tabled(rename = "Size (bytes)")]
    pub size_bytes: u64,
    #[tabled(rename = "Size (KB)")]
    pub size_kb: f64,
}

impl ContractVariant {
    pub fn from_polkavm_file(path: &Path) -> anyhow::Result<Self> {
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow::anyhow!("Invalid file name"))?;

        let without_ext = file_name
            .strip_suffix(".polkavm")
            .ok_or_else(|| anyhow::anyhow!("Not a .polkavm file"))?;

        let last_underscore = without_ext
            .rfind('_')
            .ok_or_else(|| anyhow::anyhow!("Invalid filename format"))?;

        let name = without_ext[..last_underscore].to_string();
        let variant_and_profile = &without_ext[last_underscore + 1..];

        let dot_pos = variant_and_profile
            .find('.')
            .ok_or_else(|| anyhow::anyhow!("Invalid profile format"))?;

        let variant = variant_and_profile[..dot_pos].to_string();
        let profile = variant_and_profile[dot_pos + 1..].to_string();

        let metadata = std::fs::metadata(path)?;
        let size_bytes = metadata.len();
        let size_kb = size_bytes as f64 / 1024.0;

        Ok(ContractVariant {
            name,
            variant,
            profile,
            size_bytes,
            size_kb,
        })
    }
}

pub fn collect_variants<P: AsRef<Path>>(dir: P) -> anyhow::Result<Vec<ContractVariant>> {
    let mut variants = Vec::new();

    for entry in WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "polkavm"))
    {
        match ContractVariant::from_polkavm_file(entry.path()) {
            Ok(variant) => variants.push(variant),
            Err(_) => continue,
        }
    }

    variants.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.variant.cmp(&b.variant))
            .then_with(|| a.profile.cmp(&b.profile))
    });

    Ok(variants)
}

pub fn generate_report(variants: &[ContractVariant]) -> String {
    use std::collections::BTreeMap;

    let mut report = String::new();
    report.push_str("# Binary Size Comparison Report\n\n");

    if variants.is_empty() {
        report.push_str("No variants found.\n");
        return report;
    }

    report.push_str("## Overall Comparison\n\n");
    let table = tabled::Table::new(variants);
    report.push_str(&table.to_string());
    report.push_str("\n\n");

    let mut by_contract: BTreeMap<String, Vec<&ContractVariant>> = BTreeMap::new();
    for variant in variants {
        by_contract
            .entry(variant.name.clone())
            .or_insert_with(Vec::new)
            .push(variant);
    }

    report.push_str("## Per-Contract Comparison\n\n");
    for (contract_name, contract_variants) in by_contract {
        report.push_str(&format!("### {}\n\n", contract_name));
        let table = tabled::Table::new(&contract_variants);
        report.push_str(&table.to_string());
        report.push_str("\n\n");

        let baseline = contract_variants
            .iter()
            .find(|v| v.variant == "no-alloc")
            .map(|v| v.size_bytes);

        if let Some(baseline_size) = baseline {
            report.push_str("#### Size Differences (vs no-alloc baseline)\n\n");
            let mut diffs = Vec::new();
            for variant in &contract_variants {
                if variant.variant != "no-alloc" {
                    let diff_bytes = variant.size_bytes as i64 - baseline_size as i64;
                    let diff_pct = (diff_bytes as f64 / baseline_size as f64) * 100.0;
                    diffs.push((
                        variant.variant.clone(),
                        variant.profile.clone(),
                        diff_bytes,
                        diff_pct,
                    ));
                }
            }

            if !diffs.is_empty() {
                report.push_str("| Variant | Profile | Diff (bytes) | Diff (%) |\n");
                report.push_str("|---------|---------|--------------|----------|\n");
                for (variant, profile, diff_bytes, diff_pct) in diffs {
                    report.push_str(&format!(
                        "| {} | {} | {} | {:.2}% |\n",
                        variant, profile, diff_bytes, diff_pct
                    ));
                }
                report.push_str("\n");
            }
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_parse_filename() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("my_contract_no-alloc.release.polkavm");
        fs::write(&file_path, b"test").unwrap();

        let variant = ContractVariant::from_polkavm_file(&file_path).unwrap();
        assert_eq!(variant.name, "my_contract");
        assert_eq!(variant.variant, "no-alloc");
        assert_eq!(variant.profile, "release");
        assert_eq!(variant.size_bytes, 4);
        assert_eq!(variant.size_kb, 4.0 / 1024.0);
    }

    #[test]
    fn test_parse_filename_with_underscores() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir
            .path()
            .join("my_complex_contract_with_alloc.debug.polkavm");
        fs::write(&file_path, b"test data").unwrap();

        let variant = ContractVariant::from_polkavm_file(&file_path).unwrap();
        assert_eq!(variant.name, "my_complex_contract_with");
        assert_eq!(variant.variant, "alloc");
        assert_eq!(variant.profile, "debug");
        assert_eq!(variant.size_bytes, 9);
    }

    #[test]
    fn test_collect_variants() {
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path();

        fs::write(
            temp_path.join("contract_a_no-alloc.release.polkavm"),
            b"data1",
        )
        .unwrap();
        fs::write(
            temp_path.join("contract_a_with_alloc.release.polkavm"),
            b"data2",
        )
        .unwrap();
        fs::write(
            temp_path.join("contract_b_no-alloc.debug.polkavm"),
            b"data3",
        )
        .unwrap();

        let variants = collect_variants(temp_path).unwrap();
        assert_eq!(variants.len(), 3);

        assert_eq!(variants[0].name, "contract_a");
        assert_eq!(variants[0].variant, "no-alloc");
        assert_eq!(variants[1].name, "contract_a_with");
        assert_eq!(variants[1].variant, "alloc");
        assert_eq!(variants[2].name, "contract_b");
    }

    #[test]
    fn test_generate_report() {
        let variants = vec![
            ContractVariant {
                name: "test_contract".to_string(),
                variant: "no-alloc".to_string(),
                profile: "release".to_string(),
                size_bytes: 1024,
                size_kb: 1.0,
            },
            ContractVariant {
                name: "test_contract".to_string(),
                variant: "with_alloc".to_string(),
                profile: "release".to_string(),
                size_bytes: 2048,
                size_kb: 2.0,
            },
        ];

        let report = generate_report(&variants);
        assert!(report.contains("Binary Size Comparison Report"));
        assert!(report.contains("Overall Comparison"));
        assert!(report.contains("Per-Contract Comparison"));
        assert!(report.contains("test_contract"));
        assert!(report.contains("Size Differences"));
    }
}
