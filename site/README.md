# tuck.jojomensah.com

The Tuck website. Static files in this folder, plus two things the build adds:

- `island/` — the real island, built from `index.html` and `src/` with `vite.site.config.ts`. The page frames it as `island/index.html?embed`, which loads the invented sample data in `src/demo.ts` (tracks, lyrics, clips, files, quota). The desktop app never runs that code.
- `walkthroughs/` — the two animated walkthroughs (file shelf, dictation) from `site-src/walkthroughs/`, built on the Stageshow kit (MIT, see `site-src/walkthroughs/kit/LICENSE`).

Both are gitignored and rebuilt by `bun run build:site`.

## Run it locally

```powershell
bun install
bun run build:site
python -m http.server 4173 --directory site
```

Open http://localhost:4173. After changing the island or `src/demo.ts`, `node scripts/capture-site-shots.mjs` re-takes the screenshots in `img/` from the running site.

## Cloudflare Pages

One-time setup in the Cloudflare dashboard:

1. Workers & Pages → Create → Pages → Connect to Git → `jojomensah89/tuck`.
2. Production branch `main`. Build command `bun install --frozen-lockfile && bun run build:site`. Build output directory `site`.
3. Environment variable `BUN_VERSION` = `1.4.2` (what CI uses).
4. Custom domains → add `tuck.jojomensah.com`. With `jojomensah.com` on Cloudflare DNS the CNAME is created for you.

Every merge to `main` redeploys. Pull requests get preview URLs.

## Release facts

The download button, version, size, file name and SHA-256 are read from GitHub's API at page load (`releases/latest`; the hash is GitHub's own `digest` of the installer). The values written in `index.html` are the fallback when that request fails, so update them occasionally. Releases also carry `SHA256SUMS.txt`.

## Sponsors

`sponsors.json` drives the rails beside the page (and the row above the footer on smaller screens):

```json
{
  "slots": 8,
  "priceUsd": 10,
  "checkoutUrl": "",
  "contactUrl": "https://github.com/jojomensah89/tuck/issues/new?...",
  "sponsors": [
    { "name": "Example", "url": "https://example.com", "tagline": "One short line", "logo": "img/sponsors/example.png", "until": "2026-12-31" }
  ]
}
```

- `checkoutUrl` is the payment link: the "Tuck" membership on Buy Me a Coffee ($10 a month). With it empty, "Your product here" opens `contactUrl` instead.
- A sponsor disappears by itself the day after `until`. `logo` is optional; without it the site's favicon is used.
- "N of 8 spots open" is counted from the list. Links get `?ref=tuck` and `rel="sponsored"`.

Merging a change to this file is all it takes; Cloudflare redeploys.

## Credits

The wallpaper photograph is by Billy Lam (Unsplash licence, via picsum.photos id 302).

The demo's songs are public domain, as named in the Library of Congress National Jukebox: *Danny Boy*, Ernestine Schumann-Heink, 1917 (jukebox-189331); *Aloha ʻOe*, Hawaiian Quintette, 1913 (jukebox-133081); *Auld Lang Syne*, Peerless Quartet, 1918 (jukebox-28325). Only titles, performers and the public-domain words appear; no audio is used. The artwork is drawn in `src/demo.ts`.

## Analytics

`index.html` loads the self-hosted Minilytics counter (`https://minilytics.jojomensah.com`, website ID `4o4d6s5u6q5j262l`). It runs on this page only: the island demo in `island/` and the app itself send nothing. Goals are named in `data-ml-goal` attributes (`download_click`, `github_click`, `coffee_click`, `sponsor_link_click`); the open sponsor cards are built in `main.js`, which sends `sponsor_slot_click` itself. The counter ignores localhost, so test on the deployed preview. The privacy note in `index.html` says so; keep the two in step.
