# env

# host system requirements

# Cross Compile

1. build the builder (do once)
   `docker build -t cts/rust-aarch64-linux-gnu:1.87 -f Dockerfile.toolchain .`
2. build app
   `docker build --platform linux/arm64 -t cts/docker-stat-prom:latest .`
3. create image backup
   `docker save cts/docker-stat-prom:latest | xz -vvv -T 7 > docker-stat-prom-latest.tar.xz`

# using docker image backup file

1. `sudo docker load < docker-stat-prom-latest.tar.xz`
2. `sudo docker run -d --name docker-stat-prom -p 12096:12096 -v /var/run/docker.sock:/var/run/docker.sock --restart unless-stopped --log-driver local cts/docker-stat-prom:latest`
