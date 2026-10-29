#!/usr/bin/env python3
import json
import pathlib
import re
import shutil
import subprocess
import sys
import tomllib

FORK = 'ReinaMacCredy/zeron'
UPSTREAM = 'zeronsh/zeron'
OWNER = 'ReinaMacCredy'
WHATS_NEW = pathlib.Path(__file__).resolve().parent.parent / 'crates/ui/whats_new.toml'


def version(tag):
    match = re.fullmatch(r'v?(\d+)\.(\d+)\.(\d+)', tag)
    return tuple(map(int, match.groups())) if match else None


def gh(*args):
    result = subprocess.run(['gh', *args], capture_output=True, text=True)
    if result.returncode != 0:
        raise SystemExit(f'gh {" ".join(args)} failed ({result.returncode}): {result.stderr.strip()}')
    return result.stdout


def releases(repo):
    listed = json.loads(gh('release', 'list', '-R', repo, '--limit', '200', '--json', 'tagName,isDraft'))
    return [(version(r['tagName']), r['tagName']) for r in listed if not r['isDraft'] and version(r['tagName'])]


def sections(body):
    found = {"What's Changed": [], 'New Contributors': []}
    current = None
    for line in body.replace('\r\n', '\n').split('\n'):
        if line.startswith('## '):
            current = line[3:].strip()
        elif line.startswith('**Full Changelog**'):
            current = None
        elif current in found and line.startswith('* '):
            found[current].append(re.sub(r'(?<![\w/#])#(\d+)\b', rf'https://github.com/{UPSTREAM}/pull/\1', line.rstrip()))
    return found


def main():
    if len(sys.argv) != 3:
        raise SystemExit(f'usage: {sys.argv[0]} <tag, e.g. v0.2.118> <output.md>')
    tag, out = sys.argv[1], pathlib.Path(sys.argv[2])
    new = version(tag)
    if not new:
        raise SystemExit(f'{tag} is not vX.Y.Z')
    tag = 'v' + '.'.join(map(str, new))
    if not shutil.which('gh'):
        raise SystemExit('gh is not installed; release notes need it')

    earlier = [r for r in releases(FORK) if r[0] < new]
    if not earlier:
        raise SystemExit(f'no published {FORK} release before {tag}')
    previous = max(earlier)[1]

    changed, contributors = [], []
    for _, upstream_tag in sorted(r for r in releases(UPSTREAM) if version(previous) < r[0] <= new):
        found = sections(gh('release', 'view', upstream_tag, '-R', UPSTREAM, '--json', 'body', '-q', '.body'))
        changed += found["What's Changed"]
        contributors += found['New Contributors']

    entry = next((e for e in tomllib.loads(WHATS_NEW.read_text())['entry'] if version(e['version']) == new), None)
    # upstream-* items summarize the upstream lines already listed above.
    features = [f for f in entry['features'] if not f['id'].startswith('upstream-')] if entry else []
    fork = [f"* {feature['title']} by @{OWNER}" for feature in features]
    fork += [f"* {item['text'].rstrip('.')} by @{OWNER}" for item in (entry or {}).get('improvements', [])]
    if not entry:
        print(f'::warning::{WHATS_NEW.name} has no entry for {tag[1:]}; the release body has no fork lines')

    parts = []
    if changed:
        parts.append("## What's Changed\n" + '\n'.join(changed))
    if fork:
        parts.append('## In this fork\n' + '\n'.join(fork))
    if contributors:
        parts.append('## New Contributors\n' + '\n'.join(contributors))
    parts.append(f'**Full Changelog**: https://github.com/{FORK}/compare/{previous}...{tag}')
    out.write_text('\n\n'.join(parts) + '\n')


main()
