#!/bin/bash

# This script is intended to detect race condition.

set -Eeuo pipefail

for ((i=1;i<=30;i++)); 
do
  testground run single \
   --plan=test-plan-discv5 \
   --testcase=find-node \
   --builder=docker:generic \
   --runner=local:docker \
   --instances=16 \
   --wait

  echo Finished: "$i"
done
