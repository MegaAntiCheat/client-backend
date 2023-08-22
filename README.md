# client-backend


The client app for [MAC](https://github.com/MegaAntiCheat)

## Documentation
Documentation for the API can be found at https://github.com/MegaAntiCheat/client-backend/wiki/API-Specification

## Building
1. Install Rust from https://www.rust-lang.org/tools/install
2. Navigate to inside the project folder using your favourite command line interface
3. Run with `cargo run`

Troubleshooting:
* Update rust with `rustup update`

### UI
To include the provided frontend, run the `include_ui.sh` (linux/mac) or `include_ui.bat` (windows) before building with cargo. Some dependencies will need to be installed, they can be found [here](https://github.com/MegaAntiCheat/MegaAntiCheat-UI).

Alternatively, a custom web UI can be built into the project by placing any files in the `ui` folder at compile time, and they will be served from the web interface. The web UI should include an `index.html` file as this is where the root URL will redirect to.

File are served starting from `http://127.0.0.1:3621/ui/`.

## Testing
1. Run all tests in `./tests/` with `cargo test`

## Running

For the client to interface with TF2 properly, you must have the following options somewhere in your TF2 launch options:

`-condebug -conclearlog -usercon -g15`

And you should add the following lines to your `autoexec.cfg` file or whatever config file you have setup to run on launch.
```
ip 0.0.0.0
rcon_password mac_rcon
net_start 
```

### Notes:

> The rcon_password is subject to change. If you use loopback (127.0.0.1) for the rcon IP, you are prevented from joining community servers.

> Be aware that if either the 'net_start' command or your 'autoexec' config with 'net_start' are executed multiple times during a single game, this can result in the game's networking being reset.

> The rcon command manager relies on accessing the port 27015, which causes issues if another application is using it. iTunes (AppleDeviceServices) is a notable application that binds to this port. 


You will also need to provide a SteamAPI key to the client. The client looks for configs in a config folder specified by OS.
- Windows: `C:\Users\<Your Name>\AppData\Roaming\MAC\MACClient\config`
- Mac OS: TODO - work out what this is 
- Linux: `~/.config/macclient/`

If you have not run the `client-backend` before, this config folder will not exist. You can either run the `client-backend` first or manually create the folder, then inside create a `config.yaml` file. Copy paste the following into the `config.yaml` file:
```yml
steam_api_key: "YOUR STEAM API KEY GOES HERE"
```

Then, run `cargo run` in the terminal from the root directory if you have cloned from source, OR run the executable binary.

## Contributing
Always run `cargo fmt` before submitting your Merge Request. Recommended that you also run `cargo clippy` and implement the improvements it suggests if reasonable.
