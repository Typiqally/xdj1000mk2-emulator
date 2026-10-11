"""Read-only firmware qualification through the maintained Rust shell bridge.

Owned disposable states only; three analyzed CC0 tracks are pinned by hash.
Controls use native guest input. This script never writes guest memory.
Audio is off, so the result does not qualify audio or physical hardware.
"""
import hashlib,json,queue,socket,subprocess,threading,time
from pathlib import Path
import argparse

def main():
    ROOT=Path(__file__).resolve().parents[1]
    parser=argparse.ArgumentParser(description="Qualify guest linked waveform with the pinned three-track CC0 image")
    parser.add_argument('--firmware-build',type=Path,required=True)
    parser.add_argument('--engine-selection',type=Path,required=True)
    parser.add_argument('--out',type=Path,required=True)
    args=parser.parse_args()
    OUT=args.out.resolve(); OUT.mkdir(parents=True,exist_ok=False)
    build=args.firmware_build.resolve()
    manifest=json.loads((build/'manifest.json').read_text())
    FW=build/'xdj-linked-wave-v145.UPD.research'
    if hashlib.sha256(FW.read_bytes()).hexdigest()!=manifest['sha256']: raise ValueError('Firmware manifest differs')
    MEDIA=ROOT/'output/demo-cc0-v5/cc0-demo.img'
    media_sha=hashlib.sha256(MEDIA.read_bytes()).hexdigest()
    if media_sha!='abd68e8a2ba94a3930a8b994fc92d9928f07a66b942ce47e4e49df6ecda8e264': raise ValueError('Pinned CC0 demo image differs')
    selection=json.loads(args.engine_selection.read_text()); engine=selection['engine']
    with socket.socket() as probe:
        probe.bind(('127.0.0.1',0)); port=probe.getsockname()[1]
    players=[]
    def send(player,command):
        player['process'].stdin.write(json.dumps(command)+'\n'); player['process'].stdin.flush()
    def launch(number,endpoint):
        directory=OUT/f'player{number}'; directory.mkdir(exist_ok=False)
        error=(directory/'stderr.log').open('w')
        process=subprocess.Popen([str(ROOT/'target/release/xdj-backend'),'shell','--firmware',str(FW),'--state-root',str(directory/'library'),'--engine',engine,'--qemu-data',selection['qemu_data'],'--usb-image',str(MEDIA),'--audio','off','--compile-mode','interpreter',f'--link-{endpoint}',str(port),'--link-trace','--diagnostics',str(directory)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=error,text=True,cwd=ROOT)
        player={'process':process,'directory':directory,'events':queue.Queue(),'error':error}
        players.append(player)
        def read():
            with (directory/'events.jsonl').open('w') as log:
                for line in process.stdout:
                    log.write(line); log.flush()
                    try: player['events'].put(json.loads(line))
                    except ValueError: pass
        threading.Thread(target=read,daemon=True).start()
        deadline=time.monotonic()+45
        while time.monotonic()<deadline:
            if process.poll() is not None: raise RuntimeError(f'player{number} exited: '+(directory/'stderr.log').read_text())
            try: event=player['events'].get(timeout=1)
            except queue.Empty: continue
            if event['event']=='running':
                player['state']=event['state']; print(f'player{number} running at {event["state"]}',flush=True); return player
        raise RuntimeError('Start timed out')
    try:
        a=launch(1,'listen'); b=launch(2,'connect')
        def pause(seconds):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                if any(p['process'].poll() is not None for p in players):
                    raise RuntimeError('Unexpected player exit')
                time.sleep(min(1,max(0,end-time.monotonic())))
        def snapshot(label):
            for player in players: send(player,{'action':'screenshot'})
            pause(2)
            for player in players:
                directory=player['directory']
                (directory/f'{label}.ppm').write_bytes((directory/'guest.ppm').read_bytes())
                log=Path(player['state'])/'qemu.log'
                packets=[line for line in log.read_text(errors='replace').splitlines() if 'ETHLINK' in line]
                print(f'{label} {directory.name}: {len(packets)} packets',flush=True)
        def tap(x,y):
            for player in players: send(player,{'action':'touch','x':x,'y':y,'down':True})
            pause(.15)
            for player in players: send(player,{'action':'touch','x':x,'y':y,'down':False})
            pause(2)
        pause(45)
        tap(54,260)
        pause(5)
        def button(player,name):
            send(player,{'action':'button','name':name,'down':True}); pause(.15)
            send(player,{'action':'button','name':name,'down':False}); pause(.8)
        for player in players: button(player,'browse-down')
        for player in players: button(player,'load')
        pause(3)
        button(b,'browse-down')
        for player in players: button(player,'load')
        pause(20)
        snapshot('loaded')
        def memory(label):
            for player in players:
                for name,address,size in [('module',0x0811a000,0x3800),('connection',0x0ef86000,0x4000),('display',0x0e340000,0x5000),('peers',0x0c7a0000,0x18000),('network',0x0cfc9000,0x10000),('owners',0x0da40000,0x80000),('player',0x0b5df000,0x2000)]:
                    send(player,{'action':'read-memory','address':address,'size':size,'name':label+'-'+name})
            pause(2)
        memory('loaded')
        pause(3); memory('playing-later'); snapshot('playing-later')
        for player in players:
            for _ in range(5): button(player,'browse-up')
        for zoom in range(5):
            memory('zoom-'+str(zoom));snapshot('zoom-'+str(zoom))
            for player in players: button(player,'browse-down')
            pause(1)
        button(a,'play');pause(6);memory('handover');snapshot('handover')
        send(b,{'action':'link','connected':False});pause(30);memory('unplugged');snapshot('unplugged')
        send(b,{'action':'link','connected':True});pause(45);memory('replugged');snapshot('replugged')
        button(b,'next');pause(12);memory('third-track');snapshot('third-track')

    finally:
        for player in reversed(players):
            process=player['process']
            if process.poll() is None:
                try: send(player,{'action':'release'}); send(player,{'action':'quit'}); process.stdin.close(); process.wait(timeout=15)
                except Exception: process.terminate(); process.wait(timeout=10)
            player['error'].close()
        report={'firmware':str(FW),'firmware_sha256':hashlib.sha256(FW.read_bytes()).hexdigest(),'engine':engine,'engine_sha256':hashlib.sha256(Path(engine).read_bytes()).hexdigest(),'port':port,'compile_mode':'interpreter','audio':'off','media_sha256':media_sha,'players':[{'state':p.get('state'),'exit_code':p['process'].returncode} for p in players]}
        (OUT/'identity.json').write_text(json.dumps(report,indent=2)+'\n')


if __name__ == "__main__":
    main()
