#!/bin/bash

echo "Removing old UI files"
rm -rf ui/*
touch ui/.gitkeep
rm -rf ui_temp

echo "Cloning and building UI"
git clone https://github.com/MegaAntiCheat/MegaAntiCheat-UI ui_temp
cd ui_temp

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
