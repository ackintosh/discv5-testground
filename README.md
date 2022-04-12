# test-plan-discv5

[Testground](https://github.com/testground/testground) test plans for [discv5](https://github.com/sigp/discv5).

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
    --instances=2 \
    --wait
```

## Tests

### testcase: find-node

- Narrative
  - **Warm up**
    - All nodes boot up
  - **Act I**
    - Each node calls FINDNODE query once
