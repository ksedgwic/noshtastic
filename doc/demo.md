# Noshtastic Demo Procedure

The following procedure presumes you are a rust developer and familiar
with Meshtastic radios.

This demo uses USB-serial connected Meshtastic radios.  Multiple
radios can be connected to a single computer.

## Build noshtastic

```
cargo build
```

## Connect radios

Connect multiple radios with USB cables to the computer.

## Load Test Data

**IMPORTANT** the testgw is only for initial testing.  It's important
to only ingest a few dozen nostr messages for this purpose.

Open a separate shell for each connected Meshtastic radio.  The following command specifies a standard nostr relay and filter expression to ingest test data:
```
RUST_LOG=debug,meshtastic::connections::stream_buffer=off \
cargo run -- -r wss://<relay-host> -f '{"authors":["<pubkey>"],"kinds":[1]}' -d dir0 -s /dev/ttyACM0
```
After the noshtastic node has downloaded a small number of events type ^C to kill it.

Repeat this procedure w/ the other noshtastic nodes, each in it's own
shell.  Make sure to use different `-d dir0` and `-s /dev/ttyACM0`
options for each node.

A nice demo is 20 notes in one node, 6 different notes in second node and an empty third node.

## Run the Demo

Start the first node:
```
RUST_LOG=debug,meshtastic::connections::stream_buffer=off \
cargo run -- -d dir0 -s /dev/ttyACM0
```

Wait 10 seconds until the node starts and has sent it's initial
negentropy request (which will fall on empty ears ...)

Start the second node:
```
RUST_LOG=debug,meshtastic::connections::stream_buffer=off \
cargo run -- -d dir1 -s /dev/ttyACM1
```

Start the third node:
```
RUST_LOG=debug,meshtastic::connections::stream_buffer=off \
cargo run -- -d dir2 -s /dev/ttyACM2
```

Node activity will be logged to each of the controlling terminals.

**Please be considerate of your local Meshtastic network, don't leave nodes running
when you don't need them and please don't sync a large number of nostr
notes!**
