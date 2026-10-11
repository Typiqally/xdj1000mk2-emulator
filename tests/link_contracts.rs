use std::{path::Path, process::Command};
use xdj_backend::{cli::Args, link::Link};

fn link(values: &[&str]) -> anyhow::Result<Option<Link>> {
    Link::from_args(&Args::parse(values.iter().map(|s| (*s).to_owned()))?)
}

#[test]
fn invalid_link_configuration_is_rejected() {
    for values in [
        vec!["shell", "--link-listen", "0"],
        vec!["shell", "--link-connect", "65536"],
        vec!["shell", "--link-connect", "host:9000"],
        vec!["shell", "--link-listen", "9000", "--link-connect", "9000"],
        vec!["shell", "--link-trace"],
        vec!["shell", "--link-mac", "02:00:00:00:00:01"],
    ] {
        assert!(link(&values).is_err(), "{values:?}");
    }
    for mac in [
        "ff:ff:ff:ff:ff:ff",
        "00:00:00:00:00:00",
        "02:gg:00:00:00:01",
        "02:0:00:00:00:01",
        "éé:00:00:00:00:01",
        "02:+A:00:00:00:01",
    ] {
        assert!(link(&["shell", "--link-listen", "9000", "--link-mac", mac]).is_err());
    }
    assert!(link(&["shell"]).unwrap().is_none());
}

#[test]
fn private_cable_uses_loopback_and_stable_distinct_player_addresses() {
    for endpoint in ["listen", "connect"] {
        let option = format!("--link-{endpoint}");
        let link = link(&["shell", &option, "9000"]).unwrap().unwrap();
        let first = Path::new("/private/player-one");
        let second = Path::new("/private/player-two");
        assert_eq!(link.mac_for(first), link.mac_for(first));
        assert_ne!(link.mac_for(first), link.mac_for(second));
        assert!(link.mac_for(first).starts_with("02:"));
        let mut command = Command::new("qemu");
        link.configure(&mut command, first);
        let arguments: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            arguments,
            [
                "-netdev",
                &format!("socket,id=djlink,{endpoint}=127.0.0.1:9000")
            ]
        );
    }
}

#[test]
fn explicit_identity_and_trace_override_inherited_engine_environment() {
    let link = link(&[
        "shell",
        "--link-connect",
        "65535",
        "--link-mac",
        "02:AA:BB:CC:DD:EE",
        "--link-trace",
    ])
    .unwrap()
    .unwrap();
    let mut command = Command::new("qemu");
    command.env_remove("CDJ_ETHER_MAC");
    command.env_remove("CDJ_ETHER_LINKLOG");
    link.configure(&mut command, Path::new("/private/player"));
    let environment: std::collections::BTreeMap<_, _> = command.get_envs().collect();
    assert_eq!(
        environment[std::ffi::OsStr::new("CDJ_ETHER_MAC")],
        Some(std::ffi::OsStr::new("02:aa:bb:cc:dd:ee"))
    );
    assert_eq!(
        environment[std::ffi::OsStr::new("CDJ_ETHER_LINKLOG")],
        Some(std::ffi::OsStr::new("1"))
    );
}
