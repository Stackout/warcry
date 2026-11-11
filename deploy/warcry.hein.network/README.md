# warcry.hein.network

Fan-out hub. The Mac publishes to it; phones read from it. The hub cannot press
anything: it holds no pairing token and never inspects a press, it only forwards
frames. Admission is decided on the Mac by `warcry-relay::host::Hosted`.

## Two routers, on purpose

`warcry` serves the page and the phone websocket, behind the `hearth` IdP.

`warcry-host` serves `/host`, the Mac's outbound connection, and is deliberately
*not* behind `hearth` -- the Mac cannot complete a browser login. It is protected
by `WARCRY_HUB_SECRET` instead, which the hub checks in constant time before
accepting a publisher. Its priority is higher so the more specific rule wins.

## Setup

    cp .env.example .env
    openssl rand -hex 32            # put this in .env as WARCRY_HUB_SECRET
    docker compose up -d --build

Then on the Mac, with the same secret:

    warcry-rig listen --pid <wow> --serve --sounds hack/bobber.json \
      --token-file hack/token \
      --hub wss://warcry.hein.network/host --hub-secret <secret>

`--serve` keeps the direct LAN path working at the same time; the hub is additive.
