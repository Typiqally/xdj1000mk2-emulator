"""Check/export emulator source without firmware, state or generated caches."""
from __future__ import annotations

import argparse
import hashlib
import re
import shutil
import subprocess
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[1]
PUBLIC_FILES = {
    'README.md', 'LICENSE', 'LICENSE-MIT', 'NOTICE.md', 'CONTRIBUTING.md',
    'AGENTS.md', 'Cargo.toml', 'Cargo.lock', '.gitignore', '.gitattributes',
    'docs/PUBLISHING.md', 'docs/assets/README.md',
}
PUBLIC_ARTWORK = {
    'docs/assets/readme-hero.png':
        'ca3ac216d993022325e551b0cd8c11a078fe01f5a6e2c25a7a2a555ca6165f4e',
}
SOURCE_SUFFIXES = {'.rs', '.c', '.h', '.patch', '.s', '.md', '.sh', '.py',
                   '.json', '.toml', '.yml', '.yaml', '.mk', '.build', '.lock'}
SOURCE_ROOTS = {'src', 'crates', 'native', 'tests', 'scripts', 'docs', '.github', '.agents'}
EXCLUDED_PARTS = {'target', '__pycache__', 'node_modules', '.build', 'build',
                  '.pytest_cache', '.venv', 'venv'}
MAX_SOURCE_BYTES = 2 * 1024 * 1024
SECRET_PATTERNS = (
    re.compile(rb'gh[pousr]_[A-Za-z0-9]{20,}'),
    re.compile(rb'github_pat_[A-Za-z0-9_]{20,}'),
    re.compile(rb'AKIA[0-9A-Z]{16}'),
    re.compile(rb'-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----'),
    re.compile(rb'sk-(?:proj-|live-|test-)[A-Za-z0-9_-]{20,}'),
)


def allowed(name: str) -> bool:
    path = PurePosixPath(name)
    if path.is_absolute() or '..' in path.parts or not path.parts:
        return False
    if any(part in EXCLUDED_PARTS for part in path.parts):
        return False
    if name in PUBLIC_FILES or name in PUBLIC_ARTWORK:
        return True
    if path.parts[0] in SOURCE_ROOTS:
        return (path.suffix in SOURCE_SUFFIXES or
                path.name in {'LICENSE', 'COPYING', 'Makefile', 'README', '.gitignore'})
    return False


def candidates(root: Path) -> list[str]:
    # A parent research repository must not supply the inventory for a new export.
    if (root / '.git').exists():
        result = subprocess.run(
            ['git', '-C', str(root), 'ls-files', '--cached', '--others',
             '--exclude-standard', '-z'], check=True, capture_output=True)
        return sorted(set(result.stdout.decode().split('\0')) - {''})
    return sorted(path.relative_to(root).as_posix() for path in root.rglob('*')
                  if path.is_file() or path.is_symlink())


def validate(root: Path, name: str) -> bytes:
    if not allowed(name):
        raise ValueError(f'Outside public source allowlist: {name}')
    path = root / name
    for part in [path, *path.parents]:
        if part == root:
            break
        if part.is_symlink():
            raise ValueError(f'Symlink is not publishable: {name}')
    if not path.is_file():
        raise ValueError(f'Source file is missing: {name}')
    if path.stat().st_size > MAX_SOURCE_BYTES:
        raise ValueError(f'Oversized source file: {name}')
    data = path.read_bytes()
    if name in PUBLIC_ARTWORK:
        if hashlib.sha256(data).hexdigest() != PUBLIC_ARTWORK[name]:
            raise ValueError(f'Approved artwork hash changed: {name}')
    else:
        if b'\0' in data:
            raise ValueError(f'Binary content in source file: {name}')
        data.decode('utf-8')
        if any(pattern.search(data) for pattern in SECRET_PATTERNS):
            # Report only the filename, never the matching credential.
            raise ValueError(f'Possible credential/private key: {name}')
    return data


def export(root: Path, destination: Path) -> int:
    destination = destination.absolute()
    if destination.exists() or destination.is_symlink():
        raise ValueError('Destination already exists; choose a new path')
    if root.resolve().is_relative_to(destination.resolve()):
        raise ValueError('Destination must not contain the source repository')
    names = [name for name in candidates(root) if allowed(name)]
    missing = PUBLIC_FILES - set(names)
    if missing:
        raise ValueError('Missing public files: ' + ', '.join(sorted(missing)))
    # Validate the full input before creating anything. Copy the validated bytes
    # rather than reading a second time after a concurrent source edit.
    contents = [(name, validate(root, name)) for name in names]
    destination.mkdir(parents=True)
    for name, data in contents:
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        shutil.copymode(root / name, target)
    return len(contents)


def check(root: Path) -> int:
    names = candidates(root)
    missing = PUBLIC_FILES - set(names)
    if missing:
        raise ValueError('Missing public files: ' + ', '.join(sorted(missing)))
    for name in names:
        validate(root, name)
    return len(names)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    commands.add_parser('check', help='Validate a public source checkout')
    exporting = commands.add_parser('export', help='Copy allowlisted current source')
    exporting.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    try:
        count = export(ROOT, args.out) if args.command == 'export' else check(ROOT)
    except (ValueError, OSError, subprocess.CalledProcessError) as exc:
        parser.exit(1, f'{exc}\n')
    print(f'{args.command}: {count} public source files validated')


if __name__ == '__main__':
    main()
