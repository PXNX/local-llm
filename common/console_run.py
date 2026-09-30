"""Run a flow script in its own console window and keep the window open when it ends.

Used by the GUI for runs that need typing into a console (Telegram login):
  python.exe -s common/console_run.py topvideos/make_top5.py --topic funny
"""
import runpy
import sys
import traceback
from pathlib import Path

script = Path(sys.argv[1]).resolve()
sys.argv = [str(script)] + sys.argv[2:]
sys.path.insert(0, str(script.parent))
code = 0
try:
    runpy.run_path(str(script), run_name="__main__")
except SystemExit as e:
    if isinstance(e.code, int) or e.code is None:
        code = e.code or 0
    else:
        print(e.code)
        code = 1
except BaseException:
    traceback.print_exc()
    code = 1
try:
    input(f"\nFinished (exit code {code}) - press Enter to close this window.")
except EOFError:
    pass
sys.exit(code)
