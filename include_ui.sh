#!/bin/bash

UPDATE_FLAG=0

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

    if ! command -v pnpm &> /dev/null
    then
        echo ""
        echo "pnpm could not be found, install it? (Y/n)"
        read install_pnpm

        if [[ $install_pnpm =~ ^([yY](es)?)?$ ]]
        then
        sudo npm i -g pnpm
        else
        exit
        fi
    fi


    npm exec pnpm i
    npm exec pnpm run build
    cp dist/* ../ui/

    echo "Removing temp files"
    cd ..
    rm -rf ui_temp
fi
