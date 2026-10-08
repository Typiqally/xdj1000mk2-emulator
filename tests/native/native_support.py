"""Build a private DSP library from maintained sources for component tests."""
import atexit
from pathlib import Path
import subprocess
import tempfile

_workspace = None
CORE = Path(__file__).resolve().parents[2]/'native/qemu/hw/cdj/c6x'

def cache_version():
    """C66X_JIT_CACHE_VERSION: what a batch's m.cache must hold to be loaded."""
    for line in (CORE/'c66x_core_int.h').read_text().splitlines():
        if line.startswith('#define C66X_JIT_CACHE_VERSION '):
            return int(line.split()[2])
    raise RuntimeError('C66X_JIT_CACHE_VERSION is not defined')

def core_library():
    global _workspace
    if _workspace is None:
        _workspace = tempfile.TemporaryDirectory(prefix='xdj-native-test-core-')
        atexit.register(_workspace.cleanup)
        core = Path(__file__).resolve().parents[2]/'native/qemu/hw/cdj/c6x'
        library = Path(_workspace.name)/'libc66x.so'
        result = subprocess.run(['make', '-C', str(core), 'O='+_workspace.name,
                                 str(library), str(library.with_name("c6xreplay"))], capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError('Private DSP build failed: '+result.stderr[-3000:])
    return Path(_workspace.name)/'libc66x.so'
