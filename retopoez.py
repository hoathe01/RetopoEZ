"""RetopoEZ - Maya side. Everything else happens in the standalone viewer.

Paste in Maya's Script Editor (Python) - installs/updates this file, then opens the window:

    import urllib.request, os, sys
    d = os.path.join(os.path.expanduser('~'), 'RetopoEZ'); os.makedirs(d, exist_ok=True)
    urllib.request.urlretrieve('https://raw.githubusercontent.com/hoathe01/RetopoEZ/main/retopoez.py', os.path.join(d, 'retopoez.py'))
    sys.path.insert(0, d); import retopoez; retopoez.show()

First use downloads the viewer binary (a few MB) from the latest GitHub release; after that it is reused.
"""
import os
import subprocess
import sys

import maya.cmds as cmds

REPO = 'hoathe01/RetopoEZ'
_proc = None


def _xdir():
    """Exchange folder shared with the viewer (it uses the same path)."""
    import tempfile
    d = os.path.join(tempfile.gettempdir(), 'retopoez')
    os.makedirs(d, exist_ok=True)
    return d


def _asset():
    if sys.platform.startswith('win'):
        return 'retopoez-windows-x64.exe'
    if sys.platform == 'darwin':
        return 'retopoez-macos-arm64'
    return 'retopoez-linux-x64'


def ensure_viewer():
    """Path to the viewer binary; downloads it from the latest GitHub release if missing."""
    base = os.environ.get('LOCALAPPDATA') or os.path.join(os.path.expanduser('~'), '.local', 'share')
    exe = os.path.join(base, 'RetopoEZ', _asset())
    if os.path.exists(exe):
        return exe
    import urllib.request
    os.makedirs(os.path.dirname(exe), exist_ok=True)
    url = 'https://github.com/%s/releases/latest/download/%s' % (REPO, _asset())
    part = exe + '.part'
    cmds.progressWindow(title='RetopoEZ', status='Downloading viewer (first run only)...', isInterruptable=False)
    try:
        urllib.request.urlretrieve(url, part)
    except Exception as e:
        raise RuntimeError('Could not download %s (%s). Download it manually from '
                           'https://github.com/%s/releases and put it in %s' % (_asset(), e, REPO, os.path.dirname(exe)))
    finally:
        cmds.progressWindow(endProgress=True)
    os.chmod(part, 0o755)
    os.replace(part, exe)
    return exe


def open_viewer():
    """Export the selected mesh and open (or refresh) the viewer."""
    global _proc
    if not cmds.ls(sl=True, type='transform'):
        raise RuntimeError('Select the high-poly mesh first')
    cmds.loadPlugin('objExport', quiet=True)
    path = os.path.join(_xdir(), 'mesh.obj').replace('\\', '/')
    cmds.file(path, force=True, type='OBJexport', exportSelected=True,
              options='groups=0;ptgroups=0;materials=0;smoothing=0;normals=0')
    if _proc is None or _proc.poll() is not None:  # a running viewer reloads the new file itself
        _proc = subprocess.Popen([ensure_viewer(), path])


def import_retopo():
    """Bring retopo.obj (exported from the viewer) into the scene."""
    path = os.path.join(_xdir(), 'retopo.obj').replace('\\', '/')
    if not os.path.exists(path):
        raise RuntimeError('Nothing exported yet: press "Export retopo.obj" in the viewer')
    cmds.loadPlugin('objExport', quiet=True)
    new = cmds.file(path, i=True, type='OBJ', ignoreVersion=True, returnNewNodes=True, options='mo=1')
    top = cmds.ls(new, type='transform')
    if top:
        cmds.select(cmds.rename(top[0], 'RetopoEZ_mesh'))


def show():
    win = 'retopoEZWin'
    if cmds.window(win, exists=True):
        cmds.deleteUI(win)
    cmds.window(win, title='RetopoEZ', widthHeight=(230, 80), sizeable=False)
    cmds.columnLayout(adj=True, rowSpacing=4)

    def run(fn):
        def go(*_):
            try:
                fn()
            except Exception as e:
                cmds.warning('RetopoEZ: %s' % e)
        return go

    cmds.button(label='Open viewer (selected mesh)', c=run(open_viewer))
    cmds.button(label='Import retopo mesh', c=run(import_retopo))
    cmds.showWindow(win)
