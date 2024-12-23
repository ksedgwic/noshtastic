# Noshtastic

A broadcast negentropy meshtastic nostr relay

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
