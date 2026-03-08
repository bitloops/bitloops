# Build (from the bitloops_cli directory)

cd /Users/markos/code/bitloops/cli/bitloops_cli

# Required once per environment: build-time dashboard URL config
cp config/dashboard_urls.template.json config/dashboard_urls.json
# edit config/dashboard_urls.json with real values
# build script validation runs during check/build
cargo check

cargo build

# Then run it from ANY directory

cd /path/to/some-other-repo
/Users/markos/code/bitloops/cli/bitloops_cli/target/debug/bitloops init
/Users/markos/code/bitloops/cli/bitloops_cli/target/debug/bitloops enable

# OR INSTEAD, BETTER

cargo install --path . --force

# Make sure cargo is in your PATH

# this will make the `bitloops` command available globally, so you can just run

bitloops --version

# Follow these steps

1. git init
2. create + commit a tiny initial file (README.md)
3. bitloops init
4. bitloops enable
5. chat with Claude (so hooks run and stop snapshots)
6. git commit → this one should include Bitloops-Checkpoint: ... and git branch -v ,show the checkpoints branch
