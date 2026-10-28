# NomiFun keyboard desktop integration

The four keys control NomiFun. Recording uses the computer's selected microphone;
the keyboard's onboard microphone stays disabled.

On GNOME/KDE Wayland, the default desktop window path requires XWayland. Debian
and RPM packages declare the required `xwayland` / `xorg-x11-server-Xwayland`
runtime package. AppImage cannot install host packages: install XWayland through
the distribution's package manager and enable it in the desktop session before
launching. A pure Wayland session without XWayland, or an explicit
`GDK_BACKEND=wayland` override, has not passed the keyboard window contracts.

Before deleting an AppImage, run:

```sh
sh appimage-uninstall.sh /absolute/path/NomiFun.AppImage
```

Before moving the macOS app to Trash, run:

```sh
/bin/sh /Applications/NomiFun.app/Contents/Resources/keyboard-packaging/macos-uninstall.command
```

These commands stop microphone capture and the application, then remove any
retired keyboard service and its matching login item. They preserve NomiFun's
other settings and user data. The running NomiFun GUI owns the keyboard; K1
shows or hides its main window. Quitting NomiFun disables all keyboard actions.
## AppImage running-instance boundary

Quit the running AppImage before updating or uninstalling it. A new invocation can use a different temporary mount; automatic shutdown across mounts is rejected when the OS executable identity cannot prove ownership. After the old instance exits, maintenance can continue. Ordinary fixed-path Linux installations do not have this restriction. This change has no native Linux execution evidence.
