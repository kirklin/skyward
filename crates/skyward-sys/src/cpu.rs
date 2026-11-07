//! The CPUs, run as the phone's own system runs them once it has started.
//! A recovery's init leaves them as the kernel starts them: every core at
//! its top frequency, and on Qualcomm phones none powered down while idle.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Where the kernel keeps the CPUs' frequency policies, one per cluster.
const CPUFREQ: &str = "/sys/devices/system/cpu/cpufreq";

/// The governors that set a cluster's frequency by its load, the one to
/// take first: Qualcomm's, which the phones' own boot scripts set, then the
/// kernel's own.
const GOVERNORS: [&str; 2] = ["walt", "schedutil"];

/// Qualcomm's CPU idle driver keeps idle cores from powering down until it
/// is told to let them, as the phones' own boot scripts tell it.
const SLEEP_DISABLED: &str = "/sys/devices/system/cpu/qcom_lpm/parameters/sleep_disabled";

/// Have each cluster's frequency follow its load, and let idle cores power
/// down where Qualcomm's idle driver keeps them up: what was done, for the
/// log.
pub fn run_as_system() -> Result<Vec<String>> {
    let mut policies: Vec<_> = fs::read_dir(CPUFREQ)
        .with_context(|| format!("list {CPUFREQ}"))?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<_>>()
        .with_context(|| format!("list {CPUFREQ}"))?;
    policies.retain(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("policy")));
    policies.sort();
    let mut done = Vec::new();
    for policy in &policies {
        let name = policy.file_name().expect("a policy has a name").to_string_lossy().into_owned();
        let available = fs::read_to_string(policy.join("scaling_available_governors")).with_context(|| format!("read {name}'s governors"))?;
        match governor(&available) {
            Some(g) => {
                fs::write(policy.join("scaling_governor"), g).with_context(|| format!("set {name}'s governor to {g}"))?;
                done.push(format!("{name} {g}"));
            }
            None => done.push(format!("{name} offers none of {GOVERNORS:?}, only {}", available.trim())),
        }
    }
    if Path::new(SLEEP_DISABLED).exists() {
        fs::write(SLEEP_DISABLED, "N").with_context(|| format!("write {SLEEP_DISABLED}"))?;
        done.push("idle cores power down".into());
    }
    Ok(done)
}

/// The governor to run a cluster with, of the ones `available` lists.
fn governor(available: &str) -> Option<&'static str> {
    GOVERNORS.into_iter().find(|g| available.split_whitespace().any(|a| a == *g))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cluster_runs_qualcomm_s_governor_where_it_has_it_and_the_kernel_s_otherwise() {
        assert_eq!(governor("walt conservative powersave performance schedutil \n"), Some("walt"));
        assert_eq!(governor("conservative ondemand userspace powersave performance schedutil"), Some("schedutil"));
        assert_eq!(governor("performance"), None);
        assert_eq!(governor("walty schedutilx"), None, "a name is matched whole");
    }
}
