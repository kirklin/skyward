"""Subset the UI font to the diagnostic screen's characters to fit 64 MiB.

The full recovery UI is not used by this build. Keep the variable font axes
and name/license records. Requires fonttools; does not access the network.
"""
from pathlib import Path
import shutil
from fontTools import subset
from fontTools.ttLib import TTFont

root = Path(__file__).resolve().parents[2]
dest = root / "local/bringup-fonts"
dest.mkdir(parents=True, exist_ok=True)
text = (root / "crates/skyward/src/bringup.rs").read_text() + "".join(chr(i) for i in range(32, 127))
options = subset.Options()
options.name_IDs = ["*"]
options.name_languages = ["*"]
options.name_legacy = True
font = TTFont(root / "resources/fonts/MiSansVF.ttf", recalcTimestamp=False)
s = subset.Subsetter(options=options)
s.populate(text=text)
s.subset(font)
font.save(dest / "MiSansVF.ttf")
for name in ["KirkMono-Regular.ttf", "KirkMono-OFL.txt"]:
    shutil.copyfile(root / "resources/fonts" / name, dest / name)
print("Diagnostic font bytes:", (dest / "MiSansVF.ttf").stat().st_size)
