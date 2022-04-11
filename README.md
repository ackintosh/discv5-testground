# test-plan-discv5
Testground plans for Discovery v5


```shell
$ testground run single \
    --plan=test-plan-discv5 \
    --testcase=find-peers \
    --builder=docker:generic \
    --runner=local:docker \
    --instances=2 \
    --wait
```