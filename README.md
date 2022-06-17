# Testground test plans for discv5

[![CI](https://github.com/ackintosh/discv5-testground/actions/workflows/ci.yml/badge.svg)](https://github.com/ackintosh/discv5-testground/actions/workflows/ci.yml)

This repository contains [Testground](https://github.com/testground/testground) test plans for [discv5](https://github.com/sigp/discv5).

## Getting started

```shell
# Import the test plan
$ git clone https://github.com/ackintosh/discv5-testground.git
$ testground plan import --from ./discv5-testground

# Run the test plan
$ testground run single \
    --plan=discv5-testground \
    --testcase=find-node \
    --builder=docker:generic \
    --runner=local:docker \
    --instances=5 \
    --wait
```

## Test cases

### `find-node`

In this test case, the participants construct a star topology which bootstrap node at the center, and then run the FINDNODE query. Each node run the query to test whether the node can discover all other nodes in the test case.

#### Star topology

Initially, the bootstrap node's routing table contains all the nodes' ENR in the test, and each node's routing table contains the bootstrap node's ENR only.

![star-topology](https://raw.githubusercontent.com/ackintosh/discv5-testground/b2d775a1c78ce8c76cf3e7f64eb52acee813b722/diagrams/find_nodes-star_topology.png)

### `eclipse-attack-table-poisoning`

This test case does the `ECLIPSE BY TABLE POISONING` introduced by the paper.

[Low-Resource Eclipse Attacks on Ethereum’s Peer-to-Peer Network](https://eprint.iacr.org/2018/236.pdf)

The number of `instances` is fixed to 20 in this test case. For more detail, see `composition-eclipse-attack-table-poisoning.toml`.

```shell
# Run a simulation for eclipse attack by table poisoning
testground run composition -f composition-eclipse-attack-table-poisoning.toml --wait
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
