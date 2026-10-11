# Virtual Pro DJ Link

The Rust backend can join two stock firmware players with a private Ethernet
cable. It forwards the firmware's actual Ethernet frames through QEMU's TCP
socket backend on loopback; it does not synthesize beat/status packets. This
requires an engine built from the current maintained `native/qemu/` sources.

## Launch two players

Build the host with `cargo build --locked --workspace`. The isolated engine
built for the 2026-10-10 Link check is available at the path below; the normal
engine selection was left untouched. After that artifact is removed, use the
supported `xdj-backend build-engine` command to build the current native sources
and select its engine/data paths instead.

In two terminals, from this repository, set:

```sh
FW="/absolute/path/to/XDJ1KMK2.UPD"
ENGINE="$PWD/output/link-waveform-20261010/build-root/output/engine/build/qemu-system-sh4"
DATA="$PWD/output/link-waveform-20261010/build-root/output/engine/qemu-9.1.0/pc-bios"
```

Start the listener first:

```sh
./target/debug/xdj-ui --backend "$PWD/target/debug/xdj-backend" --firmware "$FW" -- \
  --engine "$ENGINE" --qemu-data "$DATA" \
  --state-root output/my-link/player-a --audio off --compile-mode interpreter \
  --link-listen 50145 --link-trace
```

Then start the connector in the other terminal:

```sh
./target/debug/xdj-ui --backend "$PWD/target/debug/xdj-backend" --firmware "$FW" -- \
  --engine "$ENGINE" --qemu-data "$DATA" \
  --state-root output/my-link/player-b --audio off --compile-mode interpreter \
  --link-connect 50145 --link-trace
```

Always give each player a separate state root. The virtual Ethernet MAC is a
stable locally administered address derived from the canonical player state
directory. `--link-mac 02:00:00:00:00:11` can override it; use different unicast
addresses for the two players. The endpoint port must be free, between 1 and
65535, and equal on both sides. Start the listener before the connector.

Interactive demos use the [three-track CC0 export](demo-media.md) configured in
`output/desktop-defaults.json`, including its precomputed waveforms and beat
grids. Both desktop and shell launches honor that default; explicit media and
the shell's saved selection still take precedence. Do not override the export
with the generated test-tone image for waveform demonstrations.

The Player → Virtual Link Cable menu can unplug and plug the cable while running.
This submenu is enabled only when an endpoint was configured. A cable action
does not claim peer discovery. Firmware/media switches and backend restarts keep
the configured endpoint. If the listener restarts, restart the connector to
restore its TCP connection; this is a two-player transport, not a reconnecting
network service or a physical LAN bridge.

`--link-trace` logs MDIO/register activity and `ETHLINK` UDP packet summaries in
the player's `qemu.log`. It requires an endpoint. Link is off when no endpoint
flag is supplied. The socket is restricted to `127.0.0.1`.

## Automation

`xdj-backend shell` accepts the same endpoint flags. Its owned JSON bridge accepts:

```json
{"action":"link","connected":false}
{"action":"link","connected":true}
```

These use QMP `set_link` for the emulated NIC. The maintained native model reflects
carrier in PSR and Clause-22 PHY status and latches the EtherC/switch interrupts.
The socket backend's carrier changes are sampled every 100 ms of virtual time,
because that backend does not notify the NIC on accept/EOF.

## Qualification boundaries

Stock v1.45 was checked with two private states, interpreter mode and audio off.
The players exchanged discovery/keepalive and status packets with distinct IPs;
startup discovery and cable recovery evidence is recorded under
`output/link-waveform-20261010/`. Full engine/firmware hashes and process exits
are in each round's `identity.json`.

The RTL8304E model implements the switch interrupt mask/acknowledgement path
observed in stock firmware. Per-port vendor interrupt status bits, full switch
behavior, physical networking, remote library/waveform retrieval, master
handover and playing-track phase accuracy remain unqualified. Audio-off checks
do not qualify playback. The reusable waveform design example lives separately
in the firmware repository at `.design/linked-waveform-example/`; it uses
synthetic data and is not a firmware patch.

The separate firmware module now also has a passing native two-player waveform
workload. Firmware build 016 retrieves owned RGB analysis and beat grids,
renders master/local strips, follows all five native zooms, changes master on
handover, clears caches after native peer timeout, and refetches after cable
recovery. All three analyzed CC0 tracks appeared through autoplay/manual Next.
That result is tied to its exact firmware and isolated engine hashes; it does
not expand the stock engine's hardware or audio qualification above. See
[`firmware/linked_wave/README.md`](../../xdj1000mk2-custom-firmware/firmware/linked_wave/README.md)
for identities, limits and reproducible checks. The native qualifier is
`scripts/qualify_linked_wave_firmware.py`; captures remain ignored local output.
