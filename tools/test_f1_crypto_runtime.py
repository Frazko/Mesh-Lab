#!/usr/bin/env python3
"""Reproduce the F1 crypto/reducer/store suite and retain source-bound local evidence."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS = ROOT / 'artifacts'
ARTIFACTS.mkdir(exist_ok=True)

def inventory():
    files = [ROOT / 'Cargo.lock', ROOT / 'Cargo.toml', ROOT / 'rust-toolchain.toml', Path(__file__).resolve()]
    for pattern in ['crates/**/*.rs', 'crates/**/Cargo.toml', 'schema/store/*', 'schema/protocol/*', 'schema/session/*', 'vectors/session/*', 'vectors/protocol/*', 'vectors/crypto/*', 'vectors/store/*']:
        files.extend(ROOT.glob(pattern))
    return {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(set(files)) if p.is_file()}

before = inventory()
command = [str(Path.home() / '.cargo/bin/cargo'), 'test', '--workspace', '--all-features', '--locked']
log = ARTIFACTS / 'f1-crypto-runtime-tests.txt'
env = {**os.environ, 'CARGO_INCREMENTAL': '0', 'CARGO_BUILD_JOBS': '1',
       'CARGO_PROFILE_DEV_DEBUG': '0', 'CARGO_PROFILE_TEST_DEBUG': '0'}
with log.open('w') as output:
    result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
text = log.read_text()
passed_cases = sum(int(n) for n in re.findall(r'test result: ok\. (\d+) passed;', text))
stable = before == inventory()
passed = result.returncode == 0 and passed_cases > 0 and stable
report = {'passed': passed, 'passed_cases': passed_cases, 'exit_code': result.returncode,
          'sources_unchanged_during_run': stable, 'command': command,
          'log_sha256': hashlib.sha256(log.read_bytes()).hexdigest(), 'source_sha256': before,
          'scope': 'Local Rust tests including crypto KATs, lab protocol v1 certificates/envelopes/receipts, Noise XX/replay, native public-key boundary, pure send/receive reducers, SQLCipher migrations/crashes, host integration and secret-type compile-fail checks. Not complete A002/A003, mobile key-store execution, BLE or the full F1 gate.'}
(ARTIFACTS / 'f1-crypto-runtime.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps({k: report[k] for k in ['passed', 'passed_cases', 'exit_code', 'sources_unchanged_during_run']}, indent=2))
if not passed:
    print(text[-16000:])
    raise SystemExit(1)
