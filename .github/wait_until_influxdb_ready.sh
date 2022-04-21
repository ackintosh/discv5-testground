#!/bin/bash

set -Eeuo pipefail

# Check if `testground` database is created.
while ! docker exec testground-influxdb bash -c "influx --execute 'show databases' | grep [t]estground";
do
  echo 'InfluxDB is not ready, waiting...'
  sleep 1
done
