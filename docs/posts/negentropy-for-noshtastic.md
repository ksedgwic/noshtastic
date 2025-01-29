# **Negentropy for Noshtastic: A Surprisingly Good Fit**  

---

## **1. Introduction: An Unlikely Match**  
When evaluating synchronization strategies for **Noshtastic**, I initially had doubts about whether **negentropy** would work in a broadcast network with constrained frame sizes. Negentropy was originally designed for a **very different environment**:  

- **Peer-to-peer communication** (single pair, not broadcast)  
- **Error-free channels** (Noshtastic operates in a very lossy environment)  
- **Large frame sizes** (Negentropy assumes a minimum of **4KB**; Noshtastic is limited to **200 bytes**)  

Given these differences, I assumed **major modifications** would be
required to adapt negentropy to Noshtastic. However, early results
have been **surprisingly promising**.

---

## **2. Modifying Negentropy for Noshtastic**  
To test the feasibility of negentropy for Noshtastic, I made a few key modifications:  

### **Frame Size Adaptation**  
The **rust-nostr/negentropy** implementation **requires 4KB frames**. I modified it to support **200-byte frames** instead, ensuring it still functioned correctly.  
- ✅ The core algorithm **remained intact**, even with significantly smaller frame sizes.  
- 🔄 I adjusted the **number of "buckets"**, which affects how `IdList`s are sized.  

These modifications were implemented in a **fork** of `rust-nostr/negentropy`, controlled by a `tiny-frame` feature flag.

---

## **3. Rethinking the Client-Server Model**  
Traditional negentropy operates with a **clear client-server distinction**, where:  
- The **client initiates** the protocol.  
- The process **runs until synchronization is complete** (i.e., all "have" and "want" lists are resolved).  

This model **doesn’t fit Noshtastic**, where nodes **broadcast** rather than communicate in strict pairs. I made several key changes:  
- Nodes **don’t run to completion** but instead **sync incrementally** and over time achieve full content synchronization.  
- Instead of accumulating "have" and "need" lists, Noshtastic nodes **immediately broadcast** any "have" content (notes that are needed by others) as soon as it is recognized.  
- Nodes **respond to negentropy packets from any/all nodes in range**, leading to a **parallel concurrent overlapping negentropy session**.  
- All nodes assert `set_initiator`, making them effectively **clients** in every exchange.  

These changes allow negentropy to function in a **fully decentralized, broadcast-based** environment.

---

## **4. Key Takeaways & Next Steps**  
While I initially expected negentropy to require heavy modifications, it has proven to be **remarkably adaptable** to Noshtastic’s needs.  

### ✅ **What Works Well**  
- Negentropy operates **correctly** with smaller frame sizes.  
- The **modified protocol** allows for effective synchronization in a broadcast network.  

### ⚠️ **Challenges & Open Questions**  
- What further tuning is indicated?  
- Currently we only "push" notes ... would adding a request and "pulling" needed notes make sense?  

Going forward, I’ll continue refining the approach and testing its **scalability in real-world conditions**.  

---
[← Back to Blog Index](../index.html)
