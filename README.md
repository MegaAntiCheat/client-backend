# client-backend


The client app for [MAC](https://github.com/MegaAntiCheat)

## Usage steps
1. Download the latest version from [releases](https://github.com/MegaAntiCheat/client-backend/releases), or build the application yourself (if you know what you're doing)
2. Add `-condebug -conclearlog -usercon -g15` to your TF2 launch options (Right click the game in your library -> `Properties...`)
3. Add the following lines to your `autoexec.cfg` file ([learn how to find yours here](https://steamcommunity.com/sharedfiles/filedetails/?id=3112357964))
```
ip 0.0.0.0
rcon_password mac_rcon
net_start
```
4. If you use mastercomfig, you will have to put your `autoexec.cfg` inside the `overrides` folder instead ([more information](https://docs.mastercomfig.com/9.9.3/customization/custom_configs))
5. Launch TF2
6. Run the application
7. Click on the link in your console window or visit `localhost:3621` in your browser
8. Get yourself a Steam Web API key from [here](https://steamcommunity.com/dev/apikey)
9. You should get a webpage with the MAC UI in it, when you connect to a match it should show all the players in the match!

## Troubleshooting

- **Rcon connection error in the console window**
  - Ensure your `autoexec.cfg` is executing properly
  - Check step 4 of *Usage steps* if you use Mastercomfig
  - Make sure you haven't accidentally made it `autoexec.cfg.txt` (you might need to [show file extensions](https://www.howtogeek.com/205086/beginner-how-to-make-windows-show-file-extensions/))
  - When you launch TF2, open your console and look for a line that looks like `Network: IP 0.0.0.0, mode MP, dedicated No, ports 27015 SV / 27005 CL`
    - If you see this line, `autoexec.cfg` it is being executed
    - If you do not have this line in your console, your `autoexec.cfg` is not being executed
  - Restart TF2, then paste the commands `ip 0.0.0.0`, `rcon_password mac_rcon` and `net_start` into your console manually
    - Restart MAC and see if it can connect afterwards
    - If it successfully connects after that, your `autoexec.cfg` file is not executing
  - Another program may be using that port, you can try change the rcon port that MAC will use
    - This is common if you have installed iTunes before
    1. Add`-port 27069` to your TF2 launch options
    2. In the MAC UI, go to settings and change the port number to `27069`
    3. Restart TF2 and MAC
- **RCon authentication error**
  - Your rcon password is not being accepted
  - Try change the password you set in your `autoexec.cfg` and update it in the MAC UI
  - Choose a password that does not contain any spaces or special characters, a single word is fine
  - The password is required to be set but does not have to be secure as nobody else will have access to your Rcon
- **No players show up in the UI when I join a match**
  - Check for an Rcon connection error in the console window and follow the steps above
- **Missing launch options, but I'm not**
  - Run the application with the flag `--ignore-lanch-opts`
- **"No UI is bundled with this version of the app" or Error 404**
  - Use the executable from [releases](https://github.com/MegaAntiCheat/client-backend/releases)
  - If you have compiled the program yourself and *want* to compile it yourself
    - Run the appropriate `include_ui` script before compiling the application
    - This will require [Node](https://nodejs.org/en/download/package-manager) to be installed, potentially among other things
    - Make sure it runs properly and doesn't throw any errors, if it does you can delete everything in the `ui` folder and the `ui_temp` folder if it exists
- **Can't locate TF2 directory**
  - Use the command suggested in the error
- **Can't locate Steam directory**
  - If you have previously installed Steam via Flatpak, ensure there are no residual files
  - May be in `~/.var/app/`
- **I can't get an API key because I haven't bought any games**
  - Only premium steam accounts (an account that has purchased something on it) can be issued Steam Web API keys
  - You can enter an empty key to the UI to skip adding a key, the app will still provide most functionality but will not have some features (such as looking up the steam profiles of players)

## Documentation
Documentation for the API can be found at https://github.com/MegaAntiCheat/client-backend/wiki/API-Specification

Config files for the application can be found at:
- Windows: `C:\Users\<Your Name>\AppData\Roaming\MAC\MACClient\config`
- Mac OS: TODO - work out what this is, nobody games on MAC
- Linux: `~/.config/macclient/`

## Building
1. Install Rust from https://www.rust-lang.org/tools/install
2. Navigate to inside the project folder using your favourite command line interface
3. Build the UI by running the appropriate `include_ui` script, see the UI section below for more information
4. Run with `cargo run`

### UI
To include the provided frontend, run the `include_ui.sh` (linux/mac) or `include_ui.bat` (windows) before building with cargo. Some dependencies will need to be installed, they can be found [here](https://github.com/MegaAntiCheat/MegaAntiCheat-UI).

Alternatively, a custom web UI can be built into the project by placing any files in the `ui` folder at compile time, and they will be served from the web interface. The web UI should include an `index.html` file as this is where the root URL will redirect to.

File are served starting from `http://127.0.0.1:3621/ui/`.

## Contributing
Always run `cargo fmt` and resolve any issues reported by `cargo clippy` before submitting your Merge Request.
