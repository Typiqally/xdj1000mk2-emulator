use xdj_backend::cli::{Args, options};

#[test]
fn explicit_pcm_capture_directory_keeps_diagnostics_muted() {
    let dir = tempfile::tempdir().unwrap();
    let args = Args::parse([
        "shell".into(),
        "--pcm-capture".into(),
        dir.path().to_string_lossy().into_owned(),
        "--diagnostics".into(),
        dir.path().to_string_lossy().into_owned(),
        "--unmute-audio".into(),
    ])
    .unwrap();
    let runtime = options(&args, dir.path()).unwrap();
    assert_eq!(runtime.capture, Some(dir.path().canonicalize().unwrap()));
    assert!(runtime.muted);
}

#[test]
fn pcm_capture_requires_an_existing_directory_and_defaults_off() {
    let dir = tempfile::tempdir().unwrap();
    let defaults = Args::parse(["shell".into()]).unwrap();
    assert!(options(&defaults, dir.path()).unwrap().capture.is_none());
    assert!(Args::parse(["shell".into(), "--pcm-capture".into()]).is_err());
    for path in [dir.path().join("missing"), dir.path().join("file")] {
        if path.ends_with("file") {
            std::fs::write(&path, b"data").unwrap();
        }
        let args = Args::parse([
            "shell".into(),
            "--pcm-capture".into(),
            path.to_string_lossy().into_owned(),
        ])
        .unwrap();
        assert!(options(&args, dir.path()).is_err());
    }
}
