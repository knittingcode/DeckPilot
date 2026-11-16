# Stream Deck access from the Flatpak: the host udev rule

A Flatpak cannot install udev rules: they belong to the host system. With
`--device=all` the sandbox can see `/dev/hidraw*`, but the nodes are owned
by `root` with mode `0600` unless a host rule gives the logged-in user
access. The `.deb` and `.rpm` packages install the rule; the Flatpak, like
the AppImage, needs the user to install it once.

The rule is the same file the other Linux packages ship,
[`packaging/linux/70-deckpilot-streamdeck.rules`](../linux/70-deckpilot-streamdeck.rules):

```udev
# Elgato Stream Deck MK.2 (USB 0fd9:0080)
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="0fd9", ATTRS{idProduct}=="0080", TAG+="uaccess"
SUBSYSTEM=="usb", ATTRS{idVendor}=="0fd9", ATTRS{idProduct}=="0080", TAG+="uaccess"
```

`TAG+="uaccess"` makes systemd-logind give the user of the active seat an
ACL on the node; no group membership and no `MODE="0666"` are needed.

Install (on the host, not inside the Flatpak):

```sh
curl -fsSLO https://raw.githubusercontent.com/knittingcode/deckpilot/main/packaging/linux/70-deckpilot-streamdeck.rules
sudo install -m 0644 70-deckpilot-streamdeck.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger
# then unplug and replug the Stream Deck
```

Check: `getfacl /dev/hidraw*` lists `user:<you>:rw-` for the Stream Deck's
node.

What the app should do (not implemented): when a Stream Deck is present on
the bus but opening it fails with `EACCES`, show these steps instead of a
generic error. The Flathub listing (metainfo) says the rule is needed.
