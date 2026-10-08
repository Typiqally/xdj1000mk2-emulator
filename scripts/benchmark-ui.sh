#!/bin/sh
# Same 800x480, 60 Hz raw RFB workload. Measures host UI only, not emulation/audio.
set -eu
repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo"
cargo build --locked --release -p xdj-ui --bins
mkdir -p output
result=$(mktemp -d "$repo/output/ui-benchmark-XXXXXX")
mkdir -p "$result/rust"
# Only the explicitly launched UI process is sampled. The common fixture is excluded.
sample() {
    name=$1
    shift
    "$@" > "$result/$name/launcher.log" 2>&1 &
    owned_pid=$!
    trap 'kill -TERM "$owned_pid" 2>/dev/null || true; wait "$owned_pid" 2>/dev/null || true' INT TERM EXIT
    while kill -0 "$owned_pid" 2>/dev/null; do
        ps -p "$owned_pid" -o time= -o rss= >> "$result/$name/samples.txt" || true
        sleep 0.5
    done
    wait "$owned_pid"
    trap - INT TERM EXIT
}
sample rust './target/release/xdj-ui' --backend "$repo/target/release/xdj-ui-fixture" --firmware fixture --diagnostics "$result/rust" --test-script crates/ui/tests/scenarios/display-benchmark.json
ruby -rjson -e '
def p95(values);values.empty? ? nil : values.sort[((values.length-1)*0.95).round];end
report={workload:"synthetic 800x480 raw full frames at 60 Hz; same scripted controls; no firmware/DSP/audio",results:{}}
["rust"].each do |name|
  dir=File.join(ARGV[0],name); fixture=JSON.parse(File.read(File.join(dir,"fixture.json")))
  raise "#{name}: display benchmark did not deliver frames" unless fixture["frames"] >= 1680
  samples=File.readlines(File.join(dir,"samples.txt")).map do |line|
    time,rss=line.split;next unless time && rss
    cpu=time.split(":").map(&:to_f).reverse.each_with_index.sum{|v,i|v*60**i};[cpu,rss.to_i]
  end.compact
  acks=File.readlines(File.join(dir,"app-events.jsonl")).map{|s|JSON.parse(s)}.select{|e|e["event"]=="input-ack" && e.dig("command","action")=="bench-echo" && e["host_received_unix_us"] && e.dig("command","host_sent_unix_us")}
  raise "#{name}: benchmark echo samples missing" unless acks.length==100
  delivery=acks.map{|e|(e["peer_received_unix_us"]-e["command"]["host_sent_unix_us"])/1000.0}
  latency=acks.map{|e|(e["host_received_unix_us"]-e["command"]["host_sent_unix_us"])/1000.0}
  report[:results][name]={ui_cpu_seconds:samples.map(&:first).max,approx_ui_cpu_percent:samples.map(&:first).max/fixture["elapsed"]*100,peak_sampled_rss_mib:samples.map(&:last).max/1024.0,peer_frames:fixture["frames"],peer_elapsed_seconds:fixture["elapsed"],input_ack_count:latency.length,synthetic_input_delivery_p95_ms:p95(delivery),synthetic_input_roundtrip_p95_ms:p95(latency)}
end
report[:binary_bytes]={rust:File.size("target/release/xdj-ui")}
File.write(File.join(ARGV[0],"benchmark.json"),JSON.pretty_generate(report));puts JSON.pretty_generate(report)
' "$result"
printf 'Artifacts: %s\n' "$result"
