"""Run the exact standalone Go experiments embedded in the original courses."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--only', nargs='+', help='validate only these lesson IDs')
    args = parser.parse_args()
    courses = json.loads((ROOT / 'assets/curriculum.json').read_text())
    if args.only:
        missing = set(args.only) - {course['id'] for course in courses}
        if missing:
            raise SystemExit(f'Unknown lesson IDs: {sorted(missing)}')
        courses = [course for course in courses if course['id'] in args.only]
    go = shutil.which('go')
    if not go:
        raise SystemExit('Go is required to validate course experiments')
    with tempfile.TemporaryDirectory(prefix='leetdesk-curriculum-') as temporary:
        directory = Path(temporary)
        for course in courses:
            source = directory / f"{course['id']}.go"
            source.write_text(course['code'])
            result = subprocess.run([go, 'run', source], capture_output=True, text=True,
                                    timeout=60, env={**os.environ, 'GOTOOLCHAIN': 'local'})
            if result.returncode or result.stdout.strip() != course['output'].strip():
                raise SystemExit(f"FAIL {course['id']}\n{result.stderr}\n"
                                 f"expected={course['output']!r}\nactual={result.stdout!r}")
            print(f"PASS {course['id']}", flush=True)
    print(f"Validated {len(courses)} course experiments")


if __name__ == '__main__':
    main()
