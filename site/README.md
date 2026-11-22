# Bragi site

Jaspr in static mode, deployed to GitHub Pages by `.github/workflows/pages.yml` on every push to `main`
(same pattern as `WeirdBrains/weirdnet-site`). The roadmap page is generated from `../roadmap.toml`:
`python3 tool/gen_roadmap.py` (CI runs it with `--check`).

```sh
dart pub global activate jaspr_cli 0.24.0-beta.1
dart pub get
jaspr serve                                   # local
jaspr build --sitemap-domain https://weirdbrains.github.io/bragi   # github.io build, base /bragi/
```

## Hosting and the custom domain

Today: <https://weirdbrains.github.io/bragi/> (base `/bragi/`).

Custom domain `bragi.weirdbrains.com` (approved, DNS not added yet). To switch, in order:

1. DNS: add `CNAME bragi -> weirdbrains.github.io.` (and nothing else; Pages issues the certificate).
2. Set the custom domain in the repo's Pages settings (`gh api -X PUT repos/WeirdBrains/bragi/pages -f cname=bragi.weirdbrains.com`), after DNS resolves.
3. Set the repository variable `CUSTOM_DOMAIN=bragi.weirdbrains.com` and re-run the Pages workflow. The build then uses
   `SITE_BASE=/`, `SITE_URL=https://bragi.weirdbrains.com`, `SIBLING_URL=https://ymir.weirdbrains.com/` and writes a `CNAME`
   file. Switch both sites together so the cross-links work.
4. Enforce HTTPS in the Pages settings once the certificate is issued.
