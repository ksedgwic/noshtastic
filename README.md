# Noshtastic: A Geo-Specific Virtual Nostr Relay for Meshtastic

**Noshtastic** operates as a **standalone Nostr network**, designed to
function independently of internet-based Nostr relays. Unlike other
setups that rely on gatewaying nostr events to and from
internet-connected relays, Noshtastic focuses on creating a
decentralized and fully self-sufficient communication network using
only Meshtastic devices. This ensures reliable message synchronization
even in environments without internet connectivity.

```mermaid
flowchart TD
   CLIENT["`**Any Nostr Client**`"]
   style CLIENT fill:#8fbc8f,stroke:#666,stroke-width:2px,color:#000

   RELAY["`**noshtastic-relay**
   localhost nostr relay`"]
   style RELAY fill:#6c757d,stroke:#666,stroke-width:2px,color:#fff

   NOSTRDB@{ shape: lin-cyl, label: "nostrdb" }
   style NOSTRDB fill:#DAA520,stroke:#666,stroke-width:2px,color:#fff

   SYNC["`**noshtastic-sync**
   (modified negentropy)`"]
   style SYNC fill:#6c757d,stroke:#666,stroke-width:2px,color:#fff

   LINK["`**noshtastic-link**
   (encoding, fragmentation)`"]
   style LINK fill:#6c757d,stroke:#666,stroke-width:2px,color:#fff

   MESH["`meshtastic
   radio`"]
   style MESH fill:#4682B4,stroke:#666,stroke-width:2px,color:#fff

   LORA(("`LoRa`"))
   style LORA fill:#4682B4,stroke:#666,stroke-width:2px,color:#fff

   PEER0["`Peer
   Noshtastic
   Node`"]
   style PEER0 fill:#4682B4,stroke:#666,stroke-width:2px,color:#fff

   PEER1["`Peer
   Noshtastic
   Node`"]
   style PEER1 fill:#4682B4,stroke:#666,stroke-width:2px,color:#fff

   PEER2["`Peer
   Noshtastic
   Node`"]
   style PEER2 fill:#4682B4,stroke:#666,stroke-width:2px,color:#fff

   CLIENT<-->|wss|RELAY

   subgraph **Phone App**
   RELAY<-->|api|NOSTRDB
   NOSTRDB<-->|api|SYNC
   SYNC<-->|api|LINK
   end

   LINK<-->|ble,usb|MESH
   MESH<-->LORA
   LORA<-.->PEER0
   LORA<-.->PEER1
   LORA<-.->PEER2
```
Noshtastic uses negentropy-based synchronization configured using geohash-specified regions. A geohash location tag is added to events intended for Noshtastic distribution:
```
...
{
  "tags": [
    ...
    ["nosh", "9q9p1dtf1"],
    ...
  ],
}
...
```

Noshtastic relays are configured to synchronize messages for specific
regions.  For example a noshtastic relay might cover **`9q[bc89]`**
(the region including `9qb`, `9qc`, `9q8`, and `9q9`).

![Bay Area Geohash](doc/bayarea-geohash.png)

*Image source: [Geohash Explorer by Chris Hewett](https://chrishewett.com/blog/geohash-explorer/)*
