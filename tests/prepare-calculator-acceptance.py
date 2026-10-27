"""Snapshot downloaded objects and publisher metadata for opt-in VM acceptance.

Usage: python tests/prepare-calculator-acceptance.py CACHE PLAN OUTPUT
The VM re-verifies signatures and content; this tool does not authorize objects.
"""
import configparser
import pathlib
import shutil
import sys
import urllib.request

cache, plan, output = map(pathlib.Path, sys.argv[1:])
output.mkdir()
repo = output / "repo"
repo.mkdir()
shutil.copytree(cache / "objects", repo / "objects")
(repo / "config").write_text("[core]\nrepo_version=1\nmode=archive-z2\n")
for name in ["summary", "summary.sig", "flathub.flatpakrepo"]:
    request = urllib.request.Request("https://dl.flathub.org/repo/" + name, headers={"User-Agent": "flatpak-rs-acceptance"})
    with urllib.request.urlopen(request, timeout=60) as response:
        (repo / name).write_bytes(response.read())
descriptor = configparser.ConfigParser()
descriptor.optionxform = str
descriptor.read(repo / "flathub.flatpakrepo")
descriptor["Flatpak Repo"]["Url"] = "http://127.0.0.1:8080"
with (output / "fixture.flatpakrepo").open("w") as stream:
    descriptor.write(stream)
shutil.copyfile(plan, output / "plan.json")
print(output)
