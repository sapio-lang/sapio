# Sapio Command Line Interface (CLI)

The Sapio CLI is a utility tool for using different software components in
the Sapio Project.

You can use the Sapio CLI to build contracts and run other programs.

Sapio CLI reads/writes local project directories for "org.judica.sapio-cli"
based on your local system preferences. See
https://docs.rs/directories/3.0.1/directories/ for more information.

# Config

A Sapio Config file (on linux at `~/.config/sapio-cli/config.json`) is a valid JSON file that looks like:

```json
{
  "main": null,
  "testnet": null,
  "signet": null,
  "regtest": {
    "active": true,
    "api_node": {
      "url": "http://127.0.0.1:18443",
      "auth": {
        "CookieFile": "/home/<user>/.bitcoin/regtest/.cookie"
      }
    },
    "emulator_nodes": {
      "enabled": true,
      "emulators": [
        [
          "tpubD6NzVbkrYhZ4Wf398td3H8YhWBsXx9Sxa4W3cQWkNW3N3DHSNB2qtPoUMXrA6JNaPxodQfRpoZNE5tGM9iZ4xfUEFRJEJvfs8W5paUagYCE",
          "ctv.d31373.org:8367"
        ]
      ],
      "threshold": 1
    },
    "plugin_map": {
      "example": "95db1a828dd1c9ab18d431eda9f99af46b9913e818277278a60708012f1d41b3"
    }
  }
}
```

It will be populated automatically on startup if no config exists with
default values. Only one network may be active at a time, but each network
can have a defined configuration.

The command line may be used to specify a different configuration.

The emulator_nodes parameter, when enabled, uses a remote server to emulate
CheckTemplateVerify-like functionality. You can replace emulators with your
own servers (which can be started via the CLI), and you can set up emulation
to work with an arbitrary M of N of your choice. Note that this may create
issues with script lengths. You can read more about the emulator in [ctv_emulators](../ctv_emulators/README.md).

The plugin_map parameter is used to map human readable names to keys for a
plugin (you can see a plugin's key with the `cli contract load` command).
This enables contracts plugins to be dynamically linked to one another per a
user's preferences.
