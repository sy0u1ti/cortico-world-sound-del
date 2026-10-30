# Publishing

## Prepare A Release

1. Work in a separate checkout; never build over an enabled installed World.
2. Match `package.json`, `package-lock.json`, `native/Cargo.toml` and `native/Cargo.lock` versions.
3. Build on Windows x64, run typecheck, Rust/TypeScript/integration tests, the extension check and package verification.
4. Keep the npm `files` allowlist, prebuilt `dist/`, native `bin/`, license notices, icon and self-description.
5. Inspect `npm pack --dry-run --json`. Never include `.env`, `.npmrc`, recovery state, crash diagnostics, local recordings, scratch files or dependencies.
6. Create the tarball with `npm pack --ignore-scripts --pack-destination release-artifacts`. Test a production-only install in a separate directory before publication.
7. Create a GitHub release with the reviewed tarball, a ready-to-install ZIP and checksums. The source repository intentionally does not track build outputs.

Only publish a version once. Do not overwrite release assets silently, rewrite
history or publish extra empty versions to satisfy a marketplace badge.

## Official Marketplace

Authoritative instructions:

- https://github.com/Pal-AI-Lab/cortico-extensions
- https://github.com/Pal-AI-Lab/cortico-extensions/blob/main/TAGS.md
- https://github.com/Pal-AI-Lab/Cortico/blob/main/docs/extensions.md

The market installs **npm packages**. Uploading a GitHub repository or opening a
PR that edits the generated catalog does not publish an extension.

```powershell
npm login --auth-type=web --registry=https://registry.npmjs.org --browser=false
npm stage publish release-artifacts/0.1.6/cortico-world-sound-del-0.1.6.tgz --ignore-scripts --access public --tag latest --registry=https://registry.npmjs.org
```

Use an npm CLI that supports staged publishing. Approve the uploaded version
under npmjs.com **Staged Packages**, then verify the public version before
requesting a marketplace sync. A staged tarball is not a published release.

```powershell
npm view cortico-world-sound-del version --registry=https://registry.npmjs.org
gh issue create --repo Pal-AI-Lab/cortico-extensions --title "[sync] cortico-world-sound-del" --body "package: cortico-world-sound-del"
```

Complete account login and any two-factor verification yourself. Do not put
tokens in this repository or send them in issues.

After publication, the `[sync]` issue requests immediate scanning; the official
Action comments with the indexed version and closes the issue. Verify that
response or `packages/cortico-world-sound-del.json` before claiming listing.
If npm publication has not succeeded, do not open a misleading sync request.

For a full upstream check, from a compatible Cortico checkout run:

```powershell
pnpm check:extension <absolute-path-to-this-package>
```

The local `npm run check:extension` also checks the manifest, import, dry mount,
self-description and official tag vocabulary through Cortico's exported APIs,
without starting the plugin.

## Tags And Certification

The package declares `voice`, `requires-extension` and `experimental`.
It improves push-to-talk input but depends on Coopanion's desktop-pet voice
module, and hardware compatibility is not broadly qualified.

There are no arbitrary model tools, general keystroke recording, microphone
recording or third-party-account automation. The warning tags for those
capabilities do not describe this extension. Reevaluate tags if scope changes.

Ordinary inclusion is not official certification or an official-product mark.
The current certification rules require a real follow-up npm release and no
direct calls to another World, among other conditions. This first public
release is not presented as meeting those additional requirements; do not
declare a certified or official badge in package metadata.
