import rattle.runtime
from genericpath import exists
import os, pathlib
RATTLEPATH = pathlib.Path.home() / ".rattle/"
def main() -> None:
    os.makedirs(RATTLEPATH, exist_ok=True)
    if exists(RATTLEPATH / "projects.toml"):
        pass
    else:
        open(RATTLEPATH / "projects.toml", "w")
    rattle.runtime.main(RATTLEPATH / "projects.toml")
