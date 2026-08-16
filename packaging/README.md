# Packaging ruster for Arch

`pacman -Syu` upgrades ruster alongside the rest of the system. This is how.

## Why a repository and not `makepkg -si`

`makepkg -si` installs a package, and that is where pacman's interest ends: it
will never tell you a newer one exists. A *repository* is the thing `-Syu`
consults. Putting the package in one — even a local one with a single package in
it — is what turns "I built it" into "the system knows about it", so ruster shows
up in an upgrade list next to everything else, and `pacman -Qo /usr/bin/ruster`
can say which package owns that file.

## Not published, and why

The project carries no licence. That is not an oversight to route around: with
no licence, no rights are granted to anyone, so the package **must not** go to
the AUR or any public mirror. The repository built here is a directory on this
machine, reached over `file://`, and nothing leaves it.

The GitHub repository being public is consistent with that — the source is
readable, the redistribution rights are not granted. If a licence is added
later, the same `PKGBUILD` is most of what an AUR submission needs.

## First run

```sh
just package
```

That builds whatever is on the remote's default branch and writes the package
into `~/.local/share/ruster-repo`. Then add the repository to
`/etc/pacman.conf` — above `[core]`, so it wins for the names it provides:

```ini
[ruster]
SigLevel = Optional TrustAll
Server = file:///home/daimyo/.local/share/ruster-repo
```

```sh
sudo pacman -Syu
```

`SigLevel = Optional TrustAll` accepts unsigned packages. That is acceptable
*because the transport is a local directory*: nothing is fetched over a network,
so there is no third party to impersonate, and the only writer is you. It would
not be acceptable for a repository served over HTTP — that one needs a signing
key, and the entry becomes `SigLevel = Required`.

## Afterwards

```sh
git push          # the PKGBUILD tracks the remote, not the working tree
just package
sudo pacman -Syu
```

The version is `r<commit-count>.<short-hash>`, so every pushed commit sorts
newer than the installed one and appears as an upgrade. Nothing is tagged yet;
if releases are ever tagged, this becomes a `pkgver` of the tag and the `-git`
suffix goes away.

## What lands on disk

| Path | What |
| :--- | :--- |
| `/usr/bin/ruster` | the editor |
| `/usr/bin/ruster-compositor` | the compositor, built `--features udev` so it runs on a VT |
| `/usr/bin/ruster-bar` | the layer-shell bar |
| `/usr/share/ruster/compositor.lua` | the shipped config, as a *reference* |
| `/usr/share/wayland-sessions/ruster.desktop` | so a display manager offers it as a session |

The reference config is deliberately not installed to `~/.config`: the
compositor reads `$XDG_CONFIG_HOME/ruster/compositor.lua`, and a package that
wrote there would overwrite your own file on every upgrade. Copy it once:

```sh
mkdir -p ~/.config/ruster
cp /usr/share/ruster/compositor.lua ~/.config/ruster/
```

## Dependencies

`depends` was not guessed. Every entry is a library `ldd` reports against a
`--features udev` build: `libinput`, `libxkbcommon`, `seatd`, `mesa` (libgbm),
`libdrm`, `lua54`. `xorg-xwayland` is an *optional* dependency — without it the
compositor warns once and runs, with X11 clients unable to connect.

`makedepends` deliberately omits `cargo` and `rust`. This machine's Rust comes
from rustup, and the pacman packages conflict with a rustup toolchain rather
than complementing it; `prepare()` fails with a clear message if no cargo is on
`PATH`.

## The build runs the tests

`check()` runs `cargo test --workspace`, so a package is only produced from a
tree that passes. It roughly doubles the build time. `makepkg --nocheck` skips
it if you are in a hurry and know what you are shipping.
