{ pkgs }:
pkgs.runCommand "flatpak-rs-graphical-fixtures" {
  nativeBuildInputs = [ pkgs.ostree pkgs.pkg-config pkgs.wayland-scanner pkgs.pkgsStatic.stdenv.cc ];
  buildInputs = [ pkgs.pkgsStatic.wayland ];
} ''
  export HOME="$TMPDIR"
  wayland-scanner client-header ${pkgs.wayland-protocols}/share/wayland-protocols/stable/xdg-shell/xdg-shell.xml xdg-shell-client-protocol.h
  wayland-scanner private-code ${pkgs.wayland-protocols}/share/wayland-protocols/stable/xdg-shell/xdg-shell.xml xdg-shell-protocol.c
  mkdir -p app/files/bin runtime/files
  $CC -static -Wall -Wextra -I. ${./graphical-fixture.c} xdg-shell-protocol.c $(pkg-config --cflags --libs --static wayland-client) -o app/files/bin/graphical
  printf '[Application]\nname=org.example.Graphical\nruntime=org.example.Platform/x86_64/stable\ncommand=graphical\n' > app/metadata
  printf '[Runtime]\nname=org.example.Platform\n' > runtime/metadata
  ostree --repo="$out" init --mode=archive --collection-id=org.example.Graphical
  ostree --repo="$out" commit -b app/org.example.Graphical/x86_64/stable --tree=dir=app
  ostree --repo="$out" commit -b runtime/org.example.Platform/x86_64/stable --tree=dir=runtime
''
