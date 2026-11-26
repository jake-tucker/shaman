#!/usr/bin/env bash
#
# Builds a minimal app bundle around the released `shaman`, signs the bundle with the release's Developer ID while
# leaving the nested `shaman` exactly as released, notarizes and staples it, and then does the same for a disk image
# holding it (issue 781). What it finds goes to out/report.txt beside the stapled bundle, the image and the notary's
# logs.
set -euo pipefail

for secret in APPLE_DEVELOPER_ID_P12_BASE64 APPLE_DEVELOPER_ID_P12_PASSWORD APPLE_TEAM_ID \
  APPLE_API_KEY APPLE_API_KEY_ID APPLE_API_ISSUER_ID; do
  if [ -z "${!secret}" ]; then
    echo "::error::The $secret secret is not set."
    exit 1
  fi
done

source_dir=$(cd "$(dirname "$0")" && pwd)
work="$RUNNER_TEMP/probe"
out="$PWD/out"
mkdir -p "$work" "$out"
report="$out/report.txt"
say() { echo "$*" | tee -a "$report"; }
# Runs a command for the report, whatever it exits with.
show() {
  say "\$ $*"
  "$@" 2>&1 | tee -a "$report" || true
}

echo "::group::Fetch the released shaman"
gh release download "$RELEASE" --repo "$GITHUB_REPOSITORY" --pattern "$ASSET.tar.gz*" --dir "$work/download"
(cd "$work/download" && shasum -a 256 -c "$ASSET.tar.gz.sha256" && tar -xzf "$ASSET.tar.gz")
released="$work/download/$ASSET/shaman"
say "released shaman: $(shasum -a 256 "$released" | awk '{print $1}')"
show codesign --display --verbose=2 "$released"
echo "::endgroup::"

echo "::group::Build the bundle"
app="$work/stage/Shaman Probe.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Helpers"
cp "$source_dir/Info.plist" "$app/Contents/Info.plist"
swiftc -O -o "$app/Contents/MacOS/ShamanProbe" "$source_dir/main.swift"
# The same file twice, so one submission says whether either place is refused.
cp -p "$released" "$app/Contents/MacOS/shaman"
cp -p "$released" "$app/Contents/Helpers/shaman"
echo "::endgroup::"

# Says whether each nested shaman is still the released file, byte for byte.
compare() {
  local place
  for place in MacOS Helpers; do
    if cmp -s "$released" "$app/Contents/$place/shaman"; then
      say "$1: Contents/$place/shaman is byte for byte the released file"
    else
      say "$1: Contents/$place/shaman DIFFERS from the released file"
    fi
  done
}

echo "::group::Import the Developer ID"
umask 077
keychain="$RUNNER_TEMP/signing.keychain-db"
keychain_password=$(uuidgen)
echo "::add-mask::$keychain_password"
security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
base64 --decode <<< "$APPLE_DEVELOPER_ID_P12_BASE64" > "$RUNNER_TEMP/developer-id.p12"
security import "$RUNNER_TEMP/developer-id.p12" -k "$keychain" -f pkcs12 \
  -P "$APPLE_DEVELOPER_ID_P12_PASSWORD" -T /usr/bin/codesign
rm "$RUNNER_TEMP/developer-id.p12"
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" > /dev/null
search_list=("$keychain")
while IFS= read -r listed; do
  search_list+=("$listed")
done < <(security list-keychains -d user | sed -E 's/^[[:space:]]*"(.*)"$/\1/')
security list-keychains -d user -s "${search_list[@]}"
identity=$(security find-identity -v -p codesigning "$keychain" |
  awk -v team="($APPLE_TEAM_ID)\"" '/"Developer ID Application: / && index($0, team) { print $2; exit }')
if [ -z "$identity" ]; then
  echo "::error::The p12 holds no valid Developer ID Application identity for the team."
  exit 1
fi
key="$RUNNER_TEMP/notary-api-key.p8"
printf '%s\n' "$APPLE_API_KEY" > "$key"
credentials=(--key "$key" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER_ID")
umask 022
echo "::endgroup::"

echo "::group::Sign the bundle, and not the shaman in it"
# No --deep: the nested shaman keeps the signature its release gave it.
codesign --force --sign "$identity" --keychain "$keychain" --options runtime --timestamp "$app"
show codesign --display --verbose=2 "$app"
show codesign --verify --deep --strict --verbose=2 "$app"
show codesign --display --verbose=2 "$app/Contents/Helpers/shaman"
compare "after signing the bundle"
echo "::endgroup::"

# Submits a file to the notary, keeps its log, and succeeds only where it was accepted.
notarize() {
  local file=$1 name=$2 result id status
  result=$(xcrun notarytool submit "$file" "${credentials[@]}" --wait --timeout 30m --output-format json) || true
  echo "$result"
  id=$(jq -r '.id // empty' <<< "$result" 2> /dev/null || true)
  status=$(jq -r '.status // empty' <<< "$result" 2> /dev/null || true)
  if [ -n "$id" ]; then
    xcrun notarytool log "$id" "${credentials[@]}" "$out/notary-$name.json" || true
    if [ -f "$out/notary-$name.json" ]; then cat "$out/notary-$name.json"; fi
  fi
  say "notarization of the $name: ${status:-no verdict} (submission ${id:-none})"
  [ "$status" = Accepted ]
}

echo "::group::Notarize and staple the bundle"
show spctl --assess --type execute -vvv "$app"
ditto -c -k --keepParent "$app" "$work/bundle.zip"
notarize "$work/bundle.zip" bundle
show xcrun stapler staple "$app"
show xcrun stapler validate "$app"
show spctl --assess --type execute -vvv "$app"
show codesign --verify --deep --strict --verbose=2 "$app"
compare "after stapling the bundle"
ditto -c -k --keepParent "$app" "$out/ShamanProbe-stapled.zip"
echo "::endgroup::"

echo "::group::Notarize and staple a disk image holding the bundle"
image="$out/ShamanProbe.dmg"
made=false
for attempt in 1 2 3 4 5; do
  if hdiutil create -volname "Shaman Probe" -srcfolder "$work/stage" -ov -format UDZO "$image"; then
    made=true
    break
  fi
  echo "hdiutil failed on attempt $attempt"
  sleep 5
done
if [ "$made" != true ]; then
  say "disk image: hdiutil could not make one"
  exit 1
fi
codesign --force --sign "$identity" --keychain "$keychain" --timestamp "$image"
show codesign --display --verbose=2 "$image"
notarize "$image" image
show xcrun stapler staple "$image"
show xcrun stapler validate "$image"
show spctl --assess --type open --context context:primary-signature -vvv "$image"
echo "::endgroup::"
