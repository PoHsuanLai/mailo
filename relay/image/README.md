# Image relay

A token relay for mailo's "Protect mail activity" (Settings > Privacy). Remote images in a
message are fetched by this Worker, so the sender's server sees Cloudflare's address instead of
yours.

    GET /i?u=<image url>
    Authorization: Bearer <token>

It returns the image, and only if it is a PNG, JPEG, GIF or WebP under 16 MB on a public `https`
address; it follows at most three redirects and sends no cookies or Referer.

## Deploy

    cd relay/image
    npx wrangler secret put TOKEN     # any long random string
    npx wrangler deploy

In mailo: Settings > Privacy, Protect mail activity: Relay; Relay format: Token; Relay: the
Worker's address; Relay key: the token.

## Other ways

mailo also speaks the Camo format (go-camo, imgproxy), and can send images through a SOCKS5 or
HTTPS proxy such as Tor; neither needs this Worker.
