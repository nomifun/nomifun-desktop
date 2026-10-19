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

These commands stop microphone capture and the device service, then unregister
the login item. They preserve NomiFun's other settings and user data. Normal
main-app exit keeps the lightweight device service waiting for K1; use the
keyboard settings' exit-all action to stop it entirely.
