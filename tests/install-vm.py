# Executed in the NixOS test driver: only public CLI observations.
selection = "org.example.Collection org.example.Fixture x86_64 stable"
runtime = "org.example.Collection org.example.Platform stable"
install = "flatpak-rs install /etc/install-fixtures/repo " + selection + " " + runtime
# Install a reference app too, then observe it only through its own public CLI.
machine.succeed(alice("flatpak --user remote-add --no-gpg-verify fixture /etc/install-fixtures/repo"))
machine.succeed(alice("flatpak --user install --noninteractive --no-related fixture org.example.Platform//stable org.example.Fixture//stable"))
reference = machine.succeed(alice("flatpak --user list --columns=application,arch,branch,active"))
installed = json.loads(machine.succeed(alice(install)))
assert installed["app"]["commit"] == machine.succeed("cat /etc/install-fixtures/app-commit").strip()
assert installed["runtime"]["commit"] == machine.succeed("cat /etc/install-fixtures/runtime-commit").strip()
assert installed["app"]["collection"] == "org.example.Collection"
assert installed["app"]["id"] == "org.example.Fixture"
assert installed == json.loads(machine.succeed(alice("flatpak-rs inspect " + selection)))
assert installed == json.loads(machine.succeed(alice(install)))

permissions = json.loads(machine.succeed(alice(install.replace("x86_64 stable", "x86_64 permissions"))))
assert any("Context.shared" in d and "no access granted" in d for d in permissions["diagnostics"])
assert any("Context.extensions" in d for d in permissions["diagnostics"])
assert any("Future Permissions.install-hook" in d for d in permissions["diagnostics"])
assert any("Extensionless Permissions.future-access" in d for d in permissions["diagnostics"])
assert permissions == json.loads(machine.succeed(alice("flatpak-rs inspect " + selection.replace("stable", "permissions"))))

def rejected(command, diagnostic):
    code, output = machine.execute(alice(command + " 2>&1"))
    assert code == 1 and diagnostic in output, (code, output)

rejected(install.replace("x86_64 stable", "x86_64 extra-data"), "unsupported structural")
rejected("flatpak-rs inspect " + selection.replace("stable", "extra-data"), "not installed")

# Distinct branches and collections coexist; the transport path is not identity.
beta = json.loads(machine.succeed(alice(install.replace("x86_64 stable", "x86_64 beta"))))
assert beta["app"]["branch"] == "beta" and beta["runtime"] == installed["runtime"]
assert beta == json.loads(machine.succeed(alice("flatpak-rs inspect " + selection.replace("stable", "beta"))))
other_command = install.replace("/repo ", "/other ").replace("org.example.Collection", "org.example.OtherCollection")
other = json.loads(machine.succeed(alice(other_command)))
assert other["app"]["id"] == installed["app"]["id"]
assert other["app"]["collection"] != installed["app"]["collection"]
assert installed == json.loads(machine.succeed(alice(install.replace("/repo ", "/mirror "))))

for source in ["update-app", "update-runtime"]:
    rejected(install.replace("/repo ", "/" + source + " "), "unsupported update")
for branch in ["extension", "hook"]:
    rejected(install.replace("x86_64 stable", "x86_64 " + branch), "unsupported structural")
    rejected("flatpak-rs inspect " + selection.replace("stable", branch), "not installed")

rejected(install.rsplit("stable", 1)[0] + "beta", "incompatible")
rejected(install.replace("org.example.Platform", "org.example.Missing"), "runtime/org.example.Missing")
rejected(install.replace("x86_64", "aarch64"), "host-native")
rejected(install.replace("org.example.Collection", "org.example.Wrong"), "collection")
rejected(install.rsplit("org.example.Collection", 1)[0] + "org.example.Wrong org.example.Platform stable", "collection")
rejected(install.replace("org.example.Fixture", "../escape"), "invalid identity")
rejected(install.replace("/etc/install-fixtures/repo", "relative/repo"), "must be absolute")
rejected("flatpak-rs inspect " + selection.replace("stable", "missing"), "not installed")
assert installed == json.loads(machine.succeed(alice("flatpak-rs inspect " + selection)))
machine.succeed(alice("systemctl --user restart flatpak-rs.service"))
assert installed == json.loads(machine.succeed(alice("flatpak-rs inspect " + selection)))

rejected(install.replace("/repo ", "/corrupt ").replace("x86_64 stable", "x86_64 corrupt"), "checksum")
rejected("flatpak-rs inspect " + selection.replace("stable", "corrupt"), "not installed")
assert reference == machine.succeed(alice("flatpak --user list --columns=application,arch,branch,active"))

machine.succeed(alice("printf untouched > /home/alice/untouched"))
rejected(install.replace("x86_64 stable", "x86_64 symlink-metadata"), "regular file")
rejected("flatpak-rs inspect " + selection.replace("stable", "symlink-metadata"), "not installed")
linked = json.loads(machine.succeed(alice(install.replace("x86_64 stable", "x86_64 payload-symlink"))))
assert linked["app"]["branch"] == "payload-symlink"
assert machine.succeed(alice("cat /home/alice/untouched")) == "untouched"

unpublished = install.replace("x86_64 stable", "x86_64 unpublished")
rejected(unpublished.replace("org.example.Platform", "org.example.Missing"), "runtime/org.example.Missing")
rejected("flatpak-rs inspect " + selection.replace("stable", "unpublished"), "not installed")
complete = json.loads(machine.succeed(alice(unpublished)))
assert complete["runtime"] == installed["runtime"]

# Diagnostic expansion must not turn valid unknown declarations into an
# oversized, uninspectable public reply (or an implicit install failure).
many = json.loads(machine.succeed(alice(install.replace("x86_64 stable", "x86_64 many-permissions"))))
assert any("future-permission" in d for d in many["diagnostics"])
assert len(many["diagnostics"]) <= 33
assert many == json.loads(machine.succeed(alice("flatpak-rs inspect " + selection.replace("stable", "many-permissions"))))

# Configured storage paths are host configuration, not permission to follow a
# symlink into another tree. Restore the ordinary configuration afterwards.
machine.succeed(alice("mkdir /home/alice/outside; ln -s /home/alice/outside /home/alice/storage-alias"))
machine.succeed(alice("systemctl --user set-environment XDG_DATA_HOME=/home/alice/storage-alias"))
machine.succeed(alice("systemctl --user restart flatpak-rs.service"))
rejected(install, "not a real directory")
assert machine.succeed(alice("ls -A /home/alice/outside")) == ""
machine.succeed(alice("systemctl --user unset-environment XDG_DATA_HOME"))
machine.succeed(alice("systemctl --user restart flatpak-rs.service"))
assert installed == json.loads(machine.succeed(alice("flatpak-rs inspect " + selection)))
