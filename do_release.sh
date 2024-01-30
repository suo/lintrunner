set -eux
OS=$(uname)

git cliff --tag "$1" > CHANGELOG.md
if [[ "$OS" == "Linux" ]]; then
    sed -i "s/^version.*/version = \"$1\"/" Cargo.toml
elif [[ "$OS" == "Darwin" ]]; then
    sed -i '' "s/^version.*/version = \"$1\"/" Cargo.toml
fi
git commit -am "chore(release): prep for $1"
git tag "v$1"
git push
git push origin "v$1"
