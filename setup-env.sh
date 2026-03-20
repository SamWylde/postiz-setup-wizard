#!/bin/bash
# Source this file to set up the MSVC build environment for Rust/Tauri
# Usage: source setup-env.sh

export LIB="C:\\Program Files\\Microsoft Visual Studio\\18\\Community\\SDK\\ScopeCppSDK\\vc15\\SDK\\lib;C:\\Program Files\\Microsoft Visual Studio\\18\\Community\\VC\\Tools\\MSVC\\14.50.35717\\lib\\onecore\\x64;C:\\Program Files\\Microsoft Visual Studio\\18\\Community\\SDK\\ScopeCppSDK\\vc15\\VC\\lib"
export INCLUDE="C:\\Program Files\\Microsoft Visual Studio\\18\\Community\\VC\\Tools\\MSVC\\14.50.35717\\include;C:\\Program Files\\Microsoft Visual Studio\\18\\Community\\SDK\\ScopeCppSDK\\vc15\\SDK\\include\\ucrt;C:\\Program Files\\Microsoft Visual Studio\\18\\Community\\SDK\\ScopeCppSDK\\vc15\\SDK\\include\\um;C:\\Program Files\\Microsoft Visual Studio\\18\\Community\\SDK\\ScopeCppSDK\\vc15\\SDK\\include\\shared;C:\\Program Files\\Microsoft Visual Studio\\18\\Community\\SDK\\ScopeCppSDK\\vc15\\VC\\include"
export PATH="/c/Program Files/Microsoft Visual Studio/18/Community/SDK/ScopeCppSDK/vc15/SDK/bin:/c/Program Files/Microsoft Visual Studio/18/Community/VC/Tools/MSVC/14.50.35717/bin/Hostx64/x64:$PATH"

echo "MSVC build environment configured."
