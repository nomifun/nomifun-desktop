# NomiFun voice input and desktop packaging

Voice input is part of NomiFun. System shortcuts and subtitle controls share the
same recording, recognition, editing and text-polishing implementation. The
optional four-key device is disabled by default; enable its connection mode in
Settings → Voice keyboard. K1 shows/hides the subtitle while preserving text,
K2 polishes, K3 cancels, and holding/releasing K4 controls speech input. Recording
uses the selected computer microphone; onboard device audio stays disabled.
Quitting NomiFun stops its voice capability and device thread.

## macOS application lifecycle

Modern application admission and stop endpoints live in
`~/Library/Application Support/NomiFun/app-maintenance`. Optional device
ownership lives separately in `NomiFun/external-keyboard`. Normal application
startup, update and uninstall do not open old `keyboard-service` locks, read its
owner descriptors, or depend on parsing an old LaunchAgent.

Old keyboard login registration cleanup runs separately after normal startup
as a bounded advisory operation. It is never called by required update or
uninstall commands. It only targets a verified registration for this installation;
unknown, changed or inaccessible entries are skipped with diagnostics. Failure
never becomes an application startup, voice initialization, update or uninstall
prerequisite. No legacy process is started or restored.

Before moving a running macOS app to Trash, use the bundled stop command:

```sh
/bin/sh /Applications/NomiFun.app/Contents/Resources/keyboard-packaging/macos-uninstall.command
```

This asks NomiFun itself to stop. A genuinely live/unconfirmed native microphone
owner still requires stopping; optional device, shortcut and historical cleanup
errors cannot override confirmed capture release. The command preserves user
settings and data and returns without querying historical login entries. Its
success does not claim that every historical login entry was removed. Deleting a stopped app directly does not run this script.

## Linux packaging

On GNOME/KDE Wayland, the desktop window path requires XWayland. Debian/RPM
packages declare `xwayland` / `xorg-x11-server-Xwayland`; AppImage users install
XWayland with their distribution package manager. Pure Wayland without XWayland
has not passed the native window contracts.

Before deleting an AppImage, run:

```sh
sh appimage-uninstall.sh /absolute/path/NomiFun.AppImage
```

Quit a running AppImage before update/uninstall. A new invocation can use a
different temporary mount; automatic shutdown across mounts is rejected when
OS executable identity cannot prove ownership. Fixed-path Linux installations
do not have this restriction. Published Linux package rendezvous and hooks are
unchanged by the macOS runtime split; this change has no native Linux execution
evidence.
