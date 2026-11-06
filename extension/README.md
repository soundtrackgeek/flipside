# Flipside Connector

An Edge/Chrome (Manifest V3) extension that checks whether Flipside can get the album on the page you are
looking at and sends it to Flipside's Auto-get. Works on Discogs, Rate Your Music, Album Of The Year,
AllMusic and YouTube.

## Install (unpacked)

1. Open `edge://extensions`, turn on **Developer mode**, choose **Load unpacked**, and pick this `extension/` folder.
2. In Flipside open **Settings → Browser extension** and press **Copy** on the pairing token.
3. Click the extension's toolbar icon, paste the token, tick **Get albums automatically** if you want it,
   and press **Test connection**.

## How it works

- The page scraper (`sites.js`, `content.js`) finds the artist and album and shows a small Flipside pill.
- All traffic goes through the service worker (`background.js`) to `http://127.0.0.1:47615` (Flipside's local
  bridge). Flipside only accepts requests that carry the token and an extension `Origin`.
- `/check` reports Deezer (match + saved login), Usenet (best Prowlarr match) and whether Soulseek is online.
  `/get` makes Flipside open its Auto-get drawer for the album: Deezer, then Usenet, then Soulseek.
- Detection is best effort. The panel's artist and album fields are editable before you press **Get it**.

## Tests

`cd extension/tests && npm install && npm test` runs the scraper tests against small HTML fixtures.
