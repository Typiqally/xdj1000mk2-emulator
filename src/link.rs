//! A private, two-player Ethernet cable using QEMU's loopback socket transport.
use crate::{cli::Args, util};
use anyhow::{Context, Result, ensure};
use std::{path::Path, process::Command};

#[derive(Clone, Debug)]
pub struct Link {
    listen: bool,
    port: u16,
    mac: Option<String>,
    trace: bool,
}

impl Link {
    pub fn from_args(args: &Args) -> Result<Option<Self>> {
        let listen = args.value("link-listen");
        let connect = args.value("link-connect");
        ensure!(
            listen.is_none() || connect.is_none(),
            "Choose --link-listen or --link-connect"
        );
        let Some(port) = listen.or(connect) else {
            ensure!(
                args.value("link-mac").is_none() && !args.flag("link-trace"),
                "--link-mac and --link-trace require a Link endpoint"
            );
            return Ok(None);
        };
        let port: u16 = port
            .parse()
            .context("Link port must be between 1 and 65535")?;
        ensure!(port != 0, "Link port must be between 1 and 65535");
        let mac = args.value("link-mac").map(parse_mac).transpose()?;
        Ok(Some(Self {
            listen: listen.is_some(),
            port,
            mac,
            trace: args.flag("link-trace"),
        }))
    }

    pub fn mac_for(&self, state: &Path) -> String {
        self.mac.clone().unwrap_or_else(|| {
            let digest = util::hash(state.as_os_str().as_encoded_bytes());
            format!(
                "02:{}:{}:{}:{}:{}",
                &digest[0..2],
                &digest[2..4],
                &digest[4..6],
                &digest[6..8],
                &digest[8..10]
            )
        })
    }

    pub fn configure(&self, command: &mut Command, state: &Path) {
        let endpoint = if self.listen { "listen" } else { "connect" };
        command
            .args([
                "-netdev",
                &format!("socket,id=djlink,{endpoint}=127.0.0.1:{}", self.port),
            ])
            .env("CDJ_ETHER_MAC", self.mac_for(state));
        if self.trace {
            command
                .env("CDJ_ETHER_LINKLOG", "1")
                .env("CDJ_ETHER_DEBUG", "1");
        }
    }
}

fn parse_mac(value: &str) -> Result<String> {
    let parts: Vec<_> = value.split(':').collect();
    ensure!(
        parts.len() == 6
            && parts
                .iter()
                .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit())),
        "Link MAC must contain six hex octets separated by colons"
    );
    let bytes: Vec<u8> = parts
        .iter()
        .map(|p| u8::from_str_radix(p, 16))
        .collect::<std::result::Result<_, _>>()
        .context("Invalid Link MAC hex octet")?;
    ensure!(
        bytes[0] & 1 == 0 && bytes.iter().any(|b| *b != 0),
        "Link MAC must be a nonzero unicast address"
    );
    Ok(value.to_ascii_lowercase())
}
