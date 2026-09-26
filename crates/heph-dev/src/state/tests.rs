use super::images::contains_numeric_identity;
use std::fs;
use tempfile::tempdir;

#[test]
fn detects_exact_numeric_identity_field() {
    let fixture = tempdir().expect("fixture");
    let passwd = fixture.path().join("passwd");
    fs::write(&passwd, "root:x:0:0\nagent:x:10001:10001\n").expect("write");
    assert!(contains_numeric_identity(&passwd, 2, "10001").expect("read"));
    assert!(!contains_numeric_identity(&passwd, 2, "1000").expect("read"));
}
