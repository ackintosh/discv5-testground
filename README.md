# Testground test plans for discv5

[![CI](https://github.com/ackintosh/test-plan-discv5/actions/workflows/ci.yml/badge.svg)](https://github.com/ackintosh/test-plan-discv5/actions/workflows/ci.yml)

This repository contains [Testground](https://github.com/testground/testground) test plans for [discv5](https://github.com/sigp/discv5).

## Getting started

```shell
# Import the test plan
$ git clone https://github.com/ackintosh/test-plan-discv5.git
$ testground plan import --from ./test-plan-discv5

# Run the test plan
$ testground run single \
    --plan=test-plan-discv5 \
    --testcase=find-node \
    --builder=docker:generic \
    --runner=local:docker \
    --instances=5 \
    --wait
```

## Tests

:construction_worker: More testcases are in progress. :construction_worker:

### testcase: find-node

- Star topology
  - ![star-topology](https://raw.githubusercontent.com/ackintosh/test-plan-discv5/b2d775a1c78ce8c76cf3e7f64eb52acee813b722/diagrams/find_nodes-star_topology.png)
  - Bootstrap node knows all the nodes in the test.
  - Other nodes knows only Bootstrap node.
- Narrative
  - **Warm up**
    - All nodes boot up.
  - **Act I**
    - Each node calls FINDNODE to find all other nodes.
