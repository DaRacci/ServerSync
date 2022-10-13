ServerSync is a small simple tool which synronizes files between on your server.

## Installation
To install server sync you will require the cargo package manager. You can install cargo by running the following command:
```bash
curl https://sh.rustup.rs -sSf | sh
```
This will work on any distro however you may want to instead use your package manager to install cargo.

Once you have cargo installed you can install server sync by running the following command:
```bash
cargo install server_sync
```

## Usage
Required environment variables:
- `SERVER_SYNC_ENV` - The env file to load data from.
- `SERVER_SYNC_REPO` - The git repository to clone sync from. (e.g. `https://[USER]:[TOKEN]@github.com/[USER]/[REPO].git`) 
- `SERVER_SYNC_BRANCH` - The branch to sync from. 
- `SERVER_SYNC_DESTINATION` - The final destination for your files.
- `SERVER_SYNC_CONTEXTS` - A string of contexts to sync. (e.g. `prod;dev`)
- `SERVER_SYNC_REPO_STORAGE` - The location to store the git repository. (e.g. `/tmp/server_sync`)
- `UID | USER` - The user that should own the files.
- `GID | GROUP` - The group that should own the files.

To use server sync cd into the git repository you want to sync.
Once you are in the git repository you can run the following command:
```bash
server_sync
```