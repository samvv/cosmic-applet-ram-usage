# RAM Usage for Cosmic DE

This repository hosts a small applet for [COSMIC][1] that allows you to monitor your RAM usage
in real-time.

**Features**

 - Fully written in Rust
 - Configurable
     - Update interval (in milliseconds)
     - Display in bytes, kilobytes, kibibits, kilobits, and so on

## Installationa

This program is in alpha and no release has been made available just yet.

### Manual

1. Clone the repository and run `cargo build --release`.
2. Copy `target/release/cosmic-applet-ram` to `/usr/bin`
3. Copy `data/be.samvervaeck.CosmicAppletRAM.desktop` to `/usr/share/application`
4. Open COSMIC Settings, navigate to Desktop -> Panel and add the applet to the panel

## License

GPL 3.0, like COSMIC
