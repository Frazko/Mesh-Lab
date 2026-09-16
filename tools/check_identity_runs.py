#!/usr/bin/env python3
"""Validate two process launches per phone using public integration-driver reports."""
import argparse
import hashlib
import json
from pathlib import Path
import re


def validate(reports):
    if len(reports) not in (2, 4):
        raise ValueError('Provide two reports per phone (2 or 4 reports).')
    pairs = []
    for index in range(0, len(reports), 2):
        first, second = (report['identity'] for report in reports[index:index + 2])
        for item in (first, second):
            if not isinstance(item, dict):
                raise ValueError("Invalid identity report.")
            if not isinstance(item.get('fingerprint'), str) or not re.fullmatch('[0-9a-f]{64}', item['fingerprint']):
                raise ValueError('Invalid public fingerprint.')
            if type(item.get('processId')) is not int or item['processId'] <= 0:
                raise ValueError('Invalid process ID.')
            if item.get('storage') not in ('Keychain', 'Android Keystore'):
                raise ValueError('Unknown native key storage.')
        if first['processId'] == second['processId']:
            raise ValueError('Same process: this does not prove persistence after termination.')
        if first['storage'] != second['storage'] or first['fingerprint'] != second['fingerprint']:
            raise ValueError('Identity or platform changed between launches.')
        pairs.append({'storage': first['storage'], 'fingerprint': first['fingerprint'],
                      'process_ids': [first['processId'], second['processId']]})
    if len(pairs) == 2:
        if pairs[0]['storage'] == pairs[1]['storage']:
            raise ValueError('The complete gate requires both iOS and Android.')
        if pairs[0]['fingerprint'] == pairs[1]['fingerprint']:
            raise ValueError('Different phones must not share an installation identity.')
    return {'passed': True, 'both_platforms_verified': len(pairs) == 2, 'devices': pairs,
            'scope': 'Native identity persistence across distinct app processes; not peer connectivity.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('reports', type=Path, nargs='+', help='first/second launch for Android, then iOS')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.output.resolve() in {path.resolve() for path in args.reports}:
        parser.error('Output must not replace an input report.')
    try:
        raw = [path.read_bytes() for path in args.reports]
        report = validate([json.loads(content) for content in raw])
    except (ValueError, KeyError, TypeError, OSError) as error:
        parser.exit(1, f'FAIL: {error}\n')
    report['input_sha256'] = {str(path): hashlib.sha256(content).hexdigest() for path, content in zip(args.reports, raw)}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
