#!/usr/bin/env bash
cargo b --example rucompart-db

./../../target/debug/examples/rucompart-db -v debug guest [::]:8051 && echo "Guest 8051 bit the dust" &
./../../target/debug/examples/rucompart-db -v debug guest [::]:8052 && echo "Guest 8052 bit the dust" &
./../../target/debug/examples/rucompart-db -v debug guest [::]:8053 && echo "Guest 8053 bit the dust" &
./../../target/debug/examples/rucompart-db -v debug guest [::]:8054 && echo "Guest 8054 bit the dust" &
./../../target/debug/examples/rucompart-db -v debug guest [::]:8055 && echo "Guest 8055 bit the dust" &
./../../target/debug/examples/rucompart-db -v debug guest [::]:8056 && echo "Guest 8056 bit the dust" &
./../../target/debug/examples/rucompart-db -v debug guest [::]:8057 && echo "Guest 8057 bit the dust" &
./../../target/debug/examples/rucompart-db -v debug guest [::]:8058 && echo "Guest 8058 bit the dust" &
./../../target/debug/examples/rucompart-db -v debug guest [::]:8059 && echo "Guest 8059 bit the dust" &
./../../target/debug/examples/rucompart-db -v debug guest [::]:8060 && echo "Guest 8060 bit the dust" &
sleep 1s
ROCKET_PORT=8050 ./../../target/debug/examples/rucompart-db -v debug host \
    -s [::]:8051 \
    -s [::]:8052 \
    -s [::]:8053 \
    -s [::]:8054 \
    -s [::]:8055 \
    -s [::]:8056 \
    -s [::]:8057 \
    -s [::]:8058 \
    -s [::]:8059 \
    -s [::]:8060 &
export HOST=$!

trap "trap - SIGTERM && kill -- -$$" SIGINT SIGTERM EXIT

while ! curl http://127.0.0.1:8050; do
    sleep 1s
done

echo "\nWriting a few entries..."
curl -X POST --data-binary @- -o/dev/null http://127.0.0.1:8050/item-a <<EOF
{ "list": [1], "number": 123 }
EOF
curl -X POST --data-binary @- -o/dev/null http://127.0.0.1:8050/item-b <<EOF
{ "list": [2], "number": 234 }
EOF
curl -X POST --data-binary @- -o/dev/null http://127.0.0.1:8050/item-c <<EOF
{ "list": [3], "number": 345 }
EOF
curl -X POST --data-binary @- -o/dev/null http://127.0.0.1:8050/item-d <<EOF
{ "list": [4], "number": 456 }
EOF
curl -X POST --data-binary @- -o/dev/null http://127.0.0.1:8050/item-e <<EOF
{ "list": [5], "number": 567 }
EOF
curl -X POST --data-binary @- -o/dev/null http://127.0.0.1:8050/item-f <<EOF
{ "list": [1], "number": 123 }
EOF
curl -X POST --data-binary @- -o/dev/null http://127.0.0.1:8050/item-g <<EOF
{ "list": [2], "number": 234 }
EOF
curl -X POST --data-binary @- -o/dev/null http://127.0.0.1:8050/item-h <<EOF
{ "list": [3], "number": 345 }
EOF
curl -X POST --data-binary @- -o/dev/null http://127.0.0.1:8050/item-i <<EOF
{ "list": [4], "number": 456 }
EOF
curl -X POST --data-binary @- -o/dev/null http://127.0.0.1:8050/item-j <<EOF
{ "list": [5], "number": 567 }
EOF
curl -X POST --data-binary @- -o/dev/null http://127.0.0.1:8050/excluded <<EOF
{}
EOF

sleep 2s

curl -X POST --data-binary @test.rhai http://127.0.0.1:8050

kill -- -$$