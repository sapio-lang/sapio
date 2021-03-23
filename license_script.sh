#!/bin/bash

for i in $(find . -name "*.rs")
do
  if ! grep -q Copyright $i
  then
    cat copyright_notice.rs $i >$i.new && mv $i.new $i
  fi
done
