#[cfg(target_os = "macos")]
use std::fs;

#[cfg(target_os = "macos")]
#[test]
fn vision_reads_ppm_without_a_swift_compiler_and_rejects_missing_images() {
    let dir = tempfile::tempdir().unwrap();
    assert!(xdj_backend::ocr::read(&dir.path().join("missing.ppm")).is_err());
    let blank = dir.path().join("blank.ppm");
    let mut bytes = b"P6\n200 100\n255\n".to_vec();
    bytes.extend(vec![255; 200 * 100 * 3]);
    fs::write(&blank, bytes).unwrap();
    assert_eq!(
        xdj_backend::ocr::read(&blank).unwrap(),
        serde_json::json!([])
    );
    fs::write(&blank, b"invalid image").unwrap();
    assert!(xdj_backend::ocr::read(&blank).is_err());
}
