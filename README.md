# client-backend
<br>

The client app for [MAC](https://github.com/MegaAntiCheat)

## Documentation
Documentation for the API can be found at https://github.com/MegaAntiCheat/client-backend/wiki/API-Specification

## Building
1. Install Rust from https://www.rust-lang.org/tools/install
2. Navigate to inside the project folder using your favourite command line interface
3. Run with `cargo run`

Troubleshooting:
* Update rust with `rustup update`

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
> Note: the rcon_password is subject to change. If you use loopback (127.0.0.1) for the rcon IP, you are prevented from joining community servers.

You will also need to provide a SteamAPI key to the client. The client looks for configs in a config folder specified by OS.
- Windows: `C:\Users\<Your Name>\AppData\Roaming\Mega Anti Cheat\config`
- Mac OS: TODO - work out what this is 
- Linux: `~/.config/megaanticheatclient/`

If you have not run the `client-backend` before, this config folder will not exist. You can either run the `client-backend` first or manually create the folder, then inside create a `config.yaml` file. Copy paste the following into the `config.yaml` file:
```yml
steam_api_key: "YOUR STEAM API KEY GOES HERE"
```

Then, run `cargo run` in the terminal from the root directory if you have cloned from source, OR run the executable binary.

## Contributing
Always run `cargo fmt` before submitting your Merge Request. Recommended that you also run `cargo clippy` and implement the improvements it suggests if reasonable.
