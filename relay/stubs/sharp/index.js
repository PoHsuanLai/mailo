// In place of sharp, which Miniflare loads only for the Images binding (imagesLocalFetcher),
// lazily and inside a try/catch. The relay binds no Images, so nothing reaches this, and the
// LGPL libvips that the real sharp brings is never installed.
throw new Error("sharp is stubbed out: the relay uses no Images binding");
