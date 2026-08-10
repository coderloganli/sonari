use sha2::{Digest, Sha256};

use crate::app_error::AppErrorReport;

pub fn compute_app_error_fingerprint(report: &AppErrorReport) -> String {
    let mut hasher = Sha256::new();
    hasher.update(report.category.trim().as_bytes());
    hasher.update(b"|");
    hasher.update(report.message.trim().as_bytes());
    hasher.update(b"|");
    hasher.update(primary_stack_frame(&report.stack_trace).as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn primary_stack_frame(stack_trace: &str) -> String {
    stack_trace
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .chars()
        .take(512)
        .collect()
}
