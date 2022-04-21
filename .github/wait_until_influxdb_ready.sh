#!/bin/bash

set -Eeuo pipefail

influx --execute 'show databases' | grep [t]estground

while [ $? -ne 0 ]; do
  echo 'InfluxDB is not ready, waiting...'
  sleep 1
  influx --execute 'show databases' | grep [t]estground
done
