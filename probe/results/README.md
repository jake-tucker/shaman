# Results for issue 781

- `report.txt`, `notary-bundle.json`, `notary-image.json`: what run 45847051217 of this branch's workflow wrote on a
  `macos-15` runner, signing with the release's own Developer ID and notarizing with its API key.
- `keychain-on-this-mac.txt`: the scratch Server's logs from a Mac, where a bare `shaman` v0.4.1 made the keychain
  item and the `shaman` v0.4.2 nested in the bundle then read it. The fifth Server there is the control: the same
  v0.4.2 file signed again by the same team under another identifier, which the keychain did not answer.

The resolution comment on the issue says what each came to.
