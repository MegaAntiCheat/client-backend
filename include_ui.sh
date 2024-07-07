#!/usr/bin/env bash

UPDATE_FLAG=0

echo "Checking for required dependencies"

if ! command -v npm &> /dev/null
then
    
    if ! command -v node &> /dev/null
    then
        echo "" 
        echo "" 
        echo "We couldn't find npm and NodeJS, however they are required for the ui"
        echo "Install NodeJS with a package manager, or try using nvm (Node Version Manager)"
        echo ""
        echo "(should be at least version 20)"
        exit
    fi

    NODE_VERSION=$( node --version )

    echo ""
    echo "" 
    echo "We couldn't find npm, but we found NodeJS ($NODE_VERSION)"
    echo "Try looking for npm in your package manager"
    echo ""
    exit
fi

NODE_VERSION=$( node --version )
MAJOR_NODE_VERSION=$(echo "$NODE_VERSION" | cut -c "2-" | cut -c "-2")

if [ "$MAJOR_NODE_VERSION" -lt "20" ]; then
    echo ""
    echo ""
    echo "Warning: NodeJS version lower than 20 (it is $NODE_VERSION)"
    echo "This might cause errors during the build process"
    echo ""
    echo ""
fi

if ! command -v pnpm &> /dev/null
then
    echo ""
    echo "pnpm could not be found, install it? (Y/n)"
    
    # Read without -r will mangle backslashes
    read -r install_pnpm

    if [[ $install_pnpm =~ ^([yY](es)?)?$ ]]
    then
        sudo npm i -g pnpm
    else
        exit
    fi
fi

echo "Checking for UI repository"

if [ -e "ui/last_commit_hash.txt" ]; then
    echo "Reading existing commit hash"
    LAST_COMMIT_HASH=$(<ui/last_commit_hash.txt)
else
    echo "last_commit_hash.txt not found, setting to empty"
    LAST_COMMIT_HASH=""
fi

echo "Fetching latest commit hash from GitHub API"
GITHUB_HASH=$(curl -s https://api.github.com/repos/MegaAntiCheat/MegaAntiCheat-UI/git/refs/heads/main | grep sha | cut -d '"' -f 4)

if [ "$LAST_COMMIT_HASH" == "$GITHUB_HASH" ]; then
    echo "No updates to the repository."
    UPDATE_FLAG=1
fi

if [ "$UPDATE_FLAG" -eq 0 ]; then
    echo "Removing old UI files"
    rm -rf ui/*
    touch ui/.gitkeep
    rm -rf ui_temp

    echo "Cloning and building UI"
    git clone --filter=tree:0  https://github.com/MegaAntiCheat/MegaAntiCheat-UI ui_temp
    
    echo "Saving last commit hash"
    cd ui_temp || exit
    git rev-parse HEAD > ../ui/last_commit_hash.txt

    npm exec pnpm i
    npm exec pnpm run build
    mkdir -p ../ui/
    cp dist/* ../ui/

    echo "Removing temp files"
    cd ..
    rm -rf ui_temp
fi
