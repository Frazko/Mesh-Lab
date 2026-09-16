#!/usr/bin/env python3
"""Run the bounded synthetic storage scenario; keep report, remove only our fixture DBs."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CARGO = Path.home() / '.cargo/bin/cargo'
env = {**os.environ, 'CARGO_INCREMENTAL': '0', 'CARGO_BUILD_JOBS': '1',
       'CARGO_PROFILE_DEV_DEBUG': '0', 'CARGO_PROFILE_TEST_DEBUG': '0'}
subprocess.run([str(CARGO), 'build', '--locked', '-p', 'mesh-sim'], cwd=ROOT, env=env, check=True)
artifacts = ROOT / 'artifacts'
artifacts.mkdir(exist_ok=True)
fixture = Path(tempfile.mkdtemp(prefix='mesh-f1-fixture-', dir=artifacts))
try:
    result = subprocess.run([str(ROOT / 'target/debug/mesh-sim'), '--scenario',
                             str(ROOT / 'scenarios/f1-recovery.json'), '--output',
                             str(fixture / 'nodes')], cwd=ROOT, check=True, capture_output=True, text=True)
    report = json.loads(result.stdout)
    assert report['passed'] and report['durable_recipient_copies'] == report['expected_recipient_copies']
    assert all(report[field] > 0 for field in ['partitioned_contacts', 'dropped_contacts',
               'restarted_stores', 'duplicate_chunks', 'lost_local_acks', 'rejected_corrupt_chunks'])
    inputs = [ROOT / 'Cargo.lock', ROOT / 'scenarios/f1-recovery.json']
    for directory in ['mesh-types', 'mesh-object', 'mesh-store', 'mesh-replication', 'mesh-sim']:
        inputs.extend((ROOT / 'crates' / directory).rglob('*.rs'))
        inputs.append(ROOT / 'crates' / directory / 'Cargo.toml')
    inputs.extend((ROOT / 'schema/store').glob('*'))
    inputs.extend((ROOT / 'vectors/store').glob('*'))
    inputs.append(Path(__file__))
    evidence = {'result': report, 'source_sha256': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(inputs)},
                'executable_sha256': hashlib.sha256((ROOT / 'target/debug/mesh-sim').read_bytes()).hexdigest(),
                'scope': 'Host simulation with public fixture keys; local persistence acknowledgments only. Not the full F1/VS1 crypto, wire, custody or radio gate.'}
    (artifacts / 'f1-recovery.json').write_text(json.dumps(evidence, indent=2) + '\n')
    print(json.dumps(report, indent=2))
except Exception as error:
    details = f'{error}\nFixture retained at {fixture}\n'
    if isinstance(error, subprocess.CalledProcessError):
        details += (error.stdout or '') + (error.stderr or '')
    (artifacts / 'f1-failure.log').write_text(details)
    raise
else:
    shutil.rmtree(fixture)
