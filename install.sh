#!/bin/sh
set -eu

REPO="magnickolas/inf"
if [ -n "${INF_INSTALL_DIR:-}" ]; then
    INSTALL_DIR="$INF_INSTALL_DIR"
elif [ -t 0 ]; then
    printf 'Install directory [%s/.local/bin]: ' "$HOME"
    read -r reply
    INSTALL_DIR="${reply:-$HOME/.local/bin}"
else
    INSTALL_DIR="$HOME/.local/bin"
fi

die() { printf 'error: %s\n' "$1" >&2; exit 1; }

detect_target() {
    os=$(uname -s)
    arch=$(uname -m)

    case "$os" in
        Linux)  os_part="unknown-linux-musl" ;;
        Darwin) os_part="apple-darwin" ;;
        FreeBSD) os_part="unknown-freebsd" ;;
        *) die "unsupported OS: $os" ;;
    esac

    case "$arch" in
        x86_64|amd64) arch_part="x86_64" ;;
        aarch64|arm64) arch_part="aarch64" ;;
        *) die "unsupported architecture: $arch" ;;
    esac

    echo "${arch_part}-${os_part}"
}

get_latest_tag() {
    curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/'
}

main() {
    target=$(detect_target)
    tag="${1:-$(get_latest_tag)}"
    [ -z "$tag" ] && die "could not determine latest release"

    archive="inf-${target}.tar.gz"
    url="https://github.com/${REPO}/releases/download/${tag}/${archive}"

    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT

    printf 'Downloading %s...\n' "$url"
    curl -fsSL -o "$tmp/$archive" "$url"
    tar xzf "$tmp/$archive" -C "$tmp"

    mkdir -p "$INSTALL_DIR"
    install -m755 "$tmp/inf" "$INSTALL_DIR/inf"
    printf 'Installed inf to %s/inf\n' "$INSTALL_DIR"
}

main "$@"
