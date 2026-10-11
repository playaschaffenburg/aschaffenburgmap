"""Desktop-Verknuepfungen "OMSIgen TEST" (dieser Ordner) und "OMSIgen" (PROD-Ordner, sobald es einen Release gibt).

  python tools/verknuepfungen.py

tools/release.py ruft das nach jedem Release auf; nach dem Umbenennen des Ordners einfach erneut ausfuehren."""
import os
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROD = os.path.join(os.path.dirname(REPO), 'OMSIgen-Prod')


def verknuepfung(name, exe, beschreibung):
    """Verknuepfung auf dem Desktop (Windows Script Host ueber PowerShell)"""
    ps = (
        "$d=[Environment]::GetFolderPath('Desktop');"
        f"$l=(New-Object -ComObject WScript.Shell).CreateShortcut((Join-Path $d '{name}.lnk'));"
        f"$l.TargetPath='{exe}';$l.WorkingDirectory='{os.path.dirname(exe)}';$l.Description='{beschreibung}';"
        f"$l.IconLocation='{exe},0';$l.Save();Join-Path $d '{name}.lnk'"
    )
    r = subprocess.run(['powershell', '-NoProfile', '-NonInteractive', '-Command', ps], capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f'{name}: {r.stderr.strip()}')
    return r.stdout.strip()


def main():
    exe = lambda d: os.path.join(d, 'editor', 'target', 'release', 'omsi-editor.exe')
    if os.path.isfile(exe(REPO)):
        print('angelegt:', verknuepfung('OMSIgen TEST', exe(REPO), 'OMSIgen TEST - Entwicklung (Zweig main)'))
    else:
        print('TEST noch nicht gebaut:', exe(REPO))
    if os.path.isfile(exe(PROD)):
        print('angelegt:', verknuepfung('OMSIgen', exe(PROD), 'OMSIgen - Release (Zweig prod)'))
    else:
        print('PROD gibt es noch nicht (erst nach dem ersten Release):', exe(PROD))


if __name__ == '__main__':
    main()
