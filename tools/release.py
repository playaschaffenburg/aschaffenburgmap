"""OMSIgen-Release: einen getesteten Stand von TEST (Zweig main, dieser Ordner) nach PROD bringen.

  python tools/release.py 0.2.0 "Kurzbeschreibung"      # Release bauen
  python tools/release.py --pruefen                      # nur pruefen, was passieren wuerde

Ablauf:
  1. Pruefen: Zweig main, keine ungesicherten Aenderungen an versionierten Dateien (ausser den Arbeitsdateien der Karte),
     Python-Tests und Rust-Tests gruen. Die Kartendaten (Documents/Aschaffenburg) gehoeren nicht zum Release.
  2. Version eintragen (editor/Cargo.toml, pyproject.toml), CHANGELOG.md ergaenzen, Commit "Release vX.Y.Z", Tag vX.Y.Z.
  3. Zweig prod auf das Tag; PROD-Ordner (git worktree, Standard ..\\OMSIgen-Prod) auf diesen Stand.
  4. Editor im PROD-Ordner bauen mit OMSIGEN_UMGEBUNG=PROD und OMSIGEN_VERSION=X.Y.Z (Fenstertitel "OMSIgen").
  5. Push von main, prod und dem Tag nach GitHub (mit --ohne-push nicht).

TEST und PROD bearbeiten dieselben Karten; getrennt sind Code, Build, Protokolle (logs / logs-prod) und Fenstertitel."""
import argparse
import datetime
import os
import re
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROD = os.path.join(os.path.dirname(REPO), 'OMSIgen-Prod')
# Kartendaten liegen im eigenen Ordner Documents\Aschaffenburg (gemeinsam fuer TEST und PROD), nicht im Code
ARBEITSDATEIEN = []


def lauf(args, cwd=REPO, env=None, still=False):
    print('>', ' '.join(args), f'({os.path.relpath(cwd, os.path.dirname(REPO))})')
    r = subprocess.run(args, cwd=cwd, env=env, capture_output=still, text=True)
    if r.returncode != 0:
        if still:
            print(r.stdout[-3000:], r.stderr[-3000:])
        sys.exit(f'abgebrochen: {" ".join(args)} (Code {r.returncode})')
    return r.stdout if still else ''


def git(*a, cwd=REPO):
    return subprocess.run(['git', *a], cwd=cwd, capture_output=True, text=True).stdout.strip()


def pruefen():
    zweig = git('rev-parse', '--abbrev-ref', 'HEAD')
    if zweig != 'main':
        sys.exit(f'Release nur aus main (TEST), aktuell: {zweig}')
    offen = [z for z in git('status', '--porcelain', '--untracked-files=no').splitlines()
             if z[3:].strip() not in ARBEITSDATEIEN]
    if offen:
        sys.exit('nicht committete Aenderungen:\n  ' + '\n  '.join(offen))
    lauf([sys.executable, '-m', 'pytest', '-q'], still=True)
    lauf(['cargo', 'test', '--release'], cwd=os.path.join(REPO, 'editor'), still=True)
    print('Tests gruen.')


def version_setzen(v):
    for datei, muster in (('editor/Cargo.toml', r'(?m)^version = "[^"]*"'), ('pyproject.toml', r'(?m)^version = "[^"]*"')):
        p = os.path.join(REPO, datei)
        t = open(p, encoding='utf-8').read()
        t2, n = re.subn(muster, f'version = "{v}"', t, count=1)
        if n != 1:
            sys.exit(f'Version in {datei} nicht gefunden')
        open(p, 'w', encoding='utf-8', newline='').write(t2)


def changelog(v, text):
    p = os.path.join(REPO, 'CHANGELOG.md')
    alt = open(p, encoding='utf-8').read() if os.path.isfile(p) else '# OMSIgen - Versionen\n\nRelease-Staende (PROD). Entwickelt wird in TEST (Zweig main).\n'
    kopf, _, rest = alt.partition('\n## ')
    if rest.startswith('Unveroeffentlicht'):      # gesammelte TEST-Aenderungen werden Teil der neuen Version
        offen, _, rest = rest.partition('\n## ')
        inhalt = offen.split('\n', 1)[1].strip() if '\n' in offen else ''
        text = '\n\n'.join(t for t in (text, inhalt) if t)
    eintrag = f'\n## v{v} ({datetime.date.today().isoformat()})\n\n{text}\n'
    open(p, 'w', encoding='utf-8', newline='').write(kopf.rstrip('\n') + '\n' + eintrag + ('\n## ' + rest if rest else ''))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('version', nargs='?', help='z. B. 0.2.0')
    ap.add_argument('text', nargs='?', default='', help='Kurzbeschreibung fuer CHANGELOG.md')
    ap.add_argument('--pruefen', action='store_true', help='nur pruefen, nichts aendern')
    ap.add_argument('--ohne-push', action='store_true')
    ap.add_argument('--prod', default=PROD, help='PROD-Ordner (git worktree)')
    a = ap.parse_args()
    pruefen()
    if a.pruefen:
        print(f'bereit. PROD-Ordner: {a.prod} ({"vorhanden" if os.path.isdir(a.prod) else "wird angelegt"})')
        return
    if not a.version or not re.fullmatch(r'\d+\.\d+\.\d+', a.version):
        sys.exit('Version angeben, z. B. 0.2.0')
    v, tag = a.version, f'v{a.version}'
    if git('tag', '-l', tag):
        sys.exit(f'{tag} gibt es schon')
    version_setzen(v)
    changelog(v, a.text or '(ohne Beschreibung)')
    lauf(['git', 'add', 'editor/Cargo.toml', 'pyproject.toml', 'CHANGELOG.md'])
    lauf(['git', '-c', 'user.name=Claude', '-c', 'user.email=noreply@anthropic.com', 'commit', '-q', '-m',
          f'Release {tag}\n\n{a.text}\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>'])
    lauf(['git', '-c', 'user.name=Claude', '-c', 'user.email=noreply@anthropic.com', 'tag', '-a', tag, '-m', f'OMSIgen {tag}: {a.text}'])
    # PROD-Ordner: eigener Arbeitsbaum auf Zweig prod
    if not os.path.isdir(a.prod):
        if git('branch', '--list', 'prod'):
            lauf(['git', 'worktree', 'add', a.prod, 'prod'])
        else:
            lauf(['git', 'worktree', 'add', '-b', 'prod', a.prod, tag])
    lauf(['git', 'checkout', '-q', 'prod'], cwd=a.prod)
    lauf(['git', 'reset', '-q', '--hard', tag], cwd=a.prod)
    env = dict(os.environ, OMSIGEN_UMGEBUNG='PROD', OMSIGEN_VERSION=v)
    lauf(['cargo', 'build', '--release'], cwd=os.path.join(a.prod, 'editor'), env=env)
    exe = os.path.join(a.prod, 'editor', 'target', 'release', 'omsi-editor.exe')
    if not a.ohne_push:
        lauf(['git', 'push', '-q', 'origin', 'main', 'prod', tag])
    lauf([sys.executable, os.path.join(REPO, 'tools', 'verknuepfungen.py')])     # Desktop: "OMSIgen" und "OMSIgen TEST"
    print(f'\nRelease {tag} fertig. PROD-Editor: {exe}')


if __name__ == '__main__':
    main()
