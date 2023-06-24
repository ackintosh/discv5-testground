# Testground test plans for discv5

[![CI](https://github.com/ackintosh/discv5-testground/actions/workflows/ci.yml/badge.svg)](https://github.com/ackintosh/discv5-testground/actions/workflows/ci.yml)

This repository contains [Testground](https://github.com/testground/testground) test plans for [discv5](https://github.com/sigp/discv5).

## Getting started

```shell
# Import the test plan
git clone https://github.com/ackintosh/discv5-testground.git
testground plan import --from ./discv5-testground

# Run the test plan
testground run single \
  --plan=discv5-testground \
  --testcase=find-node \
  --builder=docker:generic \
  --runner=local:docker \
  --instances=5 \
  --wait
```

## Test cases

- [find-node](#find-node)
- [eclipse-attack-monopolizing-by-incoming-nodes](#eclipse-attack-monopolizing-by-incoming-nodes)
- [ip-change](#ip-change)

### [`find-node`](#test-cases)

In this test case, the participants construct a star topology which bootstrap node at the center, and then run the FINDNODE query. Each node run the query to test whether the node can discover all other nodes in the test case.

```shell
testground run single \
  --plan=discv5-testground \
  --testcase=find-node \
  --builder=docker:generic \
  --runner=local:docker \
  --instances=5 \
  --wait
```

#### Star topology

Initially, the bootstrap node's routing table contains all the nodes' ENR in the test, and each node's routing table contains the bootstrap node's ENR only.

![star-topology](https://raw.githubusercontent.com/ackintosh/discv5-testground/b2d775a1c78ce8c76cf3e7f64eb52acee813b722/diagrams/find_nodes-star_topology.png)

### [`eclipse-attack-monopolizing-by-incoming-nodes`](#test-cases)

In this test case, the attacker crafts a node id which will be inserted into the particular bucket in the victim's routing table. And then the attacker sends a query to the victim in order to let the victim add the attacker's node id to its routing table (particular bucket).

The victim's routing table will be filled with the attacker's "incoming" entries if the victim is vulnerable to the attack.

```shell
testground run composition \
  -f compositions/eclipse-attack-monopolizing-by-incoming-nodes.toml \
  --wait
```

The test case should result in success. The attackers fail to fill the victim's routing table with their node id because we limit the number of incoming nodes per bucket. See: [`eclipse-attack-monopolizing-by-incoming-nodes.toml`](https://github.com/ackintosh/discv5-testground/tree/main/compositions/eclipse-attack-monopolizing-by-incoming-nodes.toml).

If you comment out this parameter as follows and run again, the victim node emits "Table full" error and the test case results in failure, since the victim's routing bucket is full of the "incoming" attacker node ids.

```diff
    [groups.run.test_params]
-    incoming_bucket_limit = "8"
+    # incoming_bucket_limit = "8"
```

### [`ip-change`](#test-cases)

In this test plan, we would simulate the scenario where the external IP address of a node changes.

This plan runs some discv5 nodes and then one of these node changes its IP address. Note that the node doesn't change the IP address in the ENR at that time.

```shell
testground run single \
  --plan=discv5-testground \
  --testcase=ip-change \
  --builder=docker:generic \
  --runner=local:docker \
  --instances=3 \
  --wait
```

Note: currently `ping_interval` is set to 1sec as default for ease of testing. See manifest.toml.

```mermaid
sequenceDiagram
    participant Node1
    participant Node2 ... Node N
    Note over Node1: Start discv5 server
    Note over Node2 ... Node N: Start discv5 server

    rect rgb(10, 10, 10)
    Note left of Node1: They communicate with each other<br> to establish a session.
    Node1 ->> Node2 ... Node N: message
    Node2 ... Node N ->> Node1: message
    Note over Node1,Node2 ... Node N: Session established
    end

    rect rgb(10, 10, 10)
    Note left of Node1: Node1 changes its IP address<br> but doesn't update its ENR.
    Note over Node1: *** Change its IP address ***
    end

    rect rgb(10, 10, 10)
    Note left of Node1: We can observe how they behave.
    Node1 ->> Node2 ... Node N: PING
    Node2 ... Node N ->> Node1: PING
    end
```

## Metrics

Metrics are stored into the metrics store, InfluxDB. The metrics can be visualized with Grafana, bundled with Testground. 

Open Grafana (localhost:3000) and run the following query.

```sql
select
  *
from
  "discv5-testground_find-node_{run_id}"
group by
  instance_seq
```
