"""Karte im Nachbau openOMSI laden und Bilder rendern (ohne Fenster, ohne Klicken).

openOMSI (https://github.com/openOMSI-Project/openOMSI) laedt OMSI-2-Karten aus dem OMSI-Ordner und kann mit
--offscreen ein Bild schreiben. Nebenbei meldet es, wie es das Pfadnetz verstanden hat (Spuren, Verbindungen,
Konflikte an Kreuzungen, Sackgassen) - eine zweite, unabhaengige Pruefung unserer Karten.

Beispiel:
  python -m omsigen.openomsi Aschaffenburg_Live_v1 --png build/oo.png --cam 390,320,60,0,-40
Kamera: x,y,z,gier,neigung - x Ost, y Nord, z Hoehe in Kartenmetern (Kachel * 300 + lokal), gier 0 = Norden,
negative Neigung = nach unten.
"""
import argparse, glob, os, re, subprocess, sys
from .config import DEFAULT_OMSI

DEFAULT_OPENOMSI = os.path.join(os.path.expanduser('~'), 'Documents', 'OpenOmsi')

# Zeilen aus dem openOMSI-Log, die wir auswerten
PATTERNS = {
    'kacheln': r'map index: (\d+) tiles read \((\d+) unreadable\), (\d+) splines, (\d+) objects',
    'spuren': r'path network: (\d+) lanes, (\d+) links',
    'konflikte': r'path network: (\d+) conflicting lane pairs at crossings, (\d+) footpath crossings',
    'sackgassen': r'path network: (\d+) street lanes lead into a dead end',
    'fehlend': r'(\d+) unresolved',
    'vorfahrt': r'(\d+) with a \[rule\] priority',
    'ampeln': r'(\d+) light programs, (\d+) lamps',
    'ki_typen': r'(\d+) AI vehicle types in (\d+) groups',
    'ki_fahrzeuge': r'traffic: (\d+) vehicles \((\d+) of them buses\)',
}


def find_exe(base=DEFAULT_OPENOMSI):
    if base and os.path.isfile(base):
        return base
    hits = sorted(glob.glob(os.path.join(base, '**', 'openomsi.exe'), recursive=True))
    return hits[-1] if hits else None


def render(map_name, png, omsi=DEFAULT_OMSI, exe=None, cam=None, size=None, timeout=600, verkehr=0, sekunden=0):
    """-> dict(ok, png, werte, warnungen, log). map_name = Ordnername unter <OMSI>/maps"""
    exe = exe or find_exe()
    if not exe:
        raise FileNotFoundError('openomsi.exe nicht gefunden (--openomsi angeben)')
    args = [exe, '--root', omsi, '--map', f'maps/{map_name}/global.cfg', '--view', 'free', '--all',
            '--offscreen', os.path.abspath(png), '--no-menu']
    if cam:
        args += ['--cam', cam]
    if size:
        args += ['--size', size]
    if verkehr:
        args += ['--traffic', str(verkehr)]
    if sekunden:
        args += ['--drive', str(sekunden)]
    p = subprocess.run(args, cwd=os.path.dirname(exe), capture_output=True, text=True, encoding='utf-8',
                       errors='replace', timeout=timeout)
    log = p.stdout + p.stderr
    werte = {}
    for k, pat in PATTERNS.items():
        m = re.search(pat, log)
        if m:
            werte[k] = tuple(int(x) for x in m.groups())
    warn = [l.split(']', 1)[-1].strip() for l in log.splitlines() if ' WARN ' in l or ' ERROR ' in l]
    return dict(ok=p.returncode == 0 and os.path.exists(png), png=png, werte=werte, warnungen=warn, log=log)


def main(argv=None):
    ap = argparse.ArgumentParser(prog='omsigen.openomsi', description='Karte in openOMSI rendern und pruefen')
    ap.add_argument('karte', help='Kartenordner-Name unter <OMSI>/maps')
    ap.add_argument('--png', required=True, help='Bilddatei')
    ap.add_argument('--cam', help='x,y,z,gier,neigung (Kartenmeter / Grad)')
    ap.add_argument('--omsi', default=DEFAULT_OMSI)
    ap.add_argument('--openomsi', default=DEFAULT_OPENOMSI, help='openomsi.exe oder Ordner, in dem sie liegt')
    ap.add_argument('--log', help='vollstaendiges openOMSI-Log hierhin schreiben')
    ap.add_argument('--verkehr', type=int, default=0, help='so viele KI-Fahrzeuge einsetzen')
    ap.add_argument('--sekunden', type=float, default=0, help='Simulation so lange laufen lassen (s)')
    a = ap.parse_args(argv)
    r = render(a.karte, a.png, a.omsi, find_exe(a.openomsi), a.cam, verkehr=a.verkehr, sekunden=a.sekunden)
    w = r['werte']
    if 'kacheln' in w:
        print(f"openOMSI: {w['kacheln'][0]} Kacheln ({w['kacheln'][1]} unlesbar), {w['kacheln'][2]} Splines, "
              f"{w['kacheln'][3]} Objekte")
    if 'spuren' in w:
        print(f"  Pfadnetz: {w['spuren'][0]} Spuren, {w['spuren'][1]} Verbindungen")
    if 'konflikte' in w:
        print(f"  Kreuzungskonflikte (nur in Kreuzungsobjekten): {w['konflikte'][0]}, "
              f"Fussweg-Querungen {w['konflikte'][1]}")
    if 'vorfahrt' in w:
        print(f"  Spuren mit Vorfahrtsregel ([rule] priority): {w['vorfahrt'][0]}")
    if 'ampeln' in w:
        print(f"  Ampelprogramme: {w['ampeln'][0]}, Signale: {w['ampeln'][1]}")
    if 'ki_typen' in w:
        print(f"  KI-Fahrzeugtypen: {w['ki_typen'][0]} in {w['ki_typen'][1]} Gruppen")
    if 'ki_fahrzeuge' in w:
        print(f"  KI-Fahrzeuge unterwegs: {w['ki_fahrzeuge'][0]}")
    if 'sackgassen' in w:
        print(f"  Fahrspuren, die in <= 500 m in einer Sackgasse enden: {w['sackgassen'][0]}")
    for l in r['warnungen'][:10]:
        print('  Warnung:', l)
    if a.log:
        with open(a.log, 'w', encoding='utf-8') as f:
            f.write(r['log'])
    print(('Bild: ' + a.png) if r['ok'] else 'openOMSI-Lauf fehlgeschlagen (siehe --log)')
    return 0 if r['ok'] else 1


if __name__ == '__main__':
    sys.exit(main())
