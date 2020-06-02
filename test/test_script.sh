set -e

function notice {
    tput bold && tput setaf 3
    echo $@
    tput sgr0
}

function success {
    tput bold && tput setaf 2
    echo $@
    tput sgr0
}

# Ensure Proxide is built so when we run it, it doesn't need to compile
# itself and starts fast. This is important since we run it in the background.
pushd ../
cargo build

# Listen for unencrypted connections.
cargo run -- config ca --ca-cert test_ca.crt --ca-key test_ca.key --create --force
cargo run -- capture -l 5555 -t localhost:8888 -o capture.cap --ca-cert test_ca.crt --ca-key test_ca.key &
PROXIDE_PID=$!
notice "Proxide started with PID $PROXIDE_PID. If the tests fail, the process might be left behind"
popd

pushd dotnet_grpc

# Test servers withotu TLS
notice "Testing plain connection directly through Proxide"
dotnet run -- --connect localhost:5555 --server-port 8888
success ...OK

# Test servers with RSA key
notice "Testing TLS connection directly through Proxide with RSA cert"
openssl req -x509 -newkey rsa:2048 -keyout test_server.key -out test_server.crt -days 365 -nodes -subj '/CN=localhost'
dotnet run -- --connect localhost:5555 \
    --server-port 8888 \
    --ca-cert ../../test_ca.crt \
    --server-cert test_server.crt \
    --server-key test_server.key
success ...OK

# Test servers with EC key
notice "Testing TLS connection directly through Proxide with EC cert"
openssl ecparam -genkey -name prime256v1 -out test_server.key
openssl req -new -sha256 -key test_server.key -out test_server.csr -subj '/CN=localhost'
openssl req -x509 -sha256 -days 365 -key test_server.key -in test_server.csr -out test_server.crt
dotnet run -- --connect localhost:5555 \
    --server-port 8888 \
    --ca-cert ../../test_ca.crt \
    --server-cert test_server.crt \
    --server-key test_server.key
success ...OK

# Test CONNECT proxy.
notice "Testing plain connection using Proxide as a CONNECT proxy"
http_proxy="http://localhost:5555" dotnet run -- --connect localhost:8888 \
    --server-port 8888
success ...OK

# Test CONNECT proxy with TLS
notice "Testing TLS connection using Proxide as a CONNECT proxy"
https_proxy=http://localhost:5555 dotnet run -- --connect localhost:8888 \
    --server-port 8888 \
    --ca-cert ../../test_ca.crt \
    --server-cert test_server.crt \
    --server-key test_server.key
success ...OK

popd

kill $PROXIDE_PID
sleep 0.1
notice "Proxide PID $PROXIDE_PID killed"
success "Tests OK!"

