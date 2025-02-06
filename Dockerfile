# # Copyright 2024 The Nerve Lab
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
# http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#
FROM ubuntu:22.04

LABEL maintainer="The nerve lab"
LABEL description="hippius Network Node"

COPY ./target/release/hippius /usr/local/bin/

RUN useradd -m -u 5000 -U -s /bin/sh -d /hippius hippius && \
	mkdir -p /data /hippius/.local/share && \
	chown -R hippius:hippius /data && \
	ln -s /data /hippius/.local/share/hippius && \
	# unclutter and minimize the attack surface
	rm -rf /usr/bin /usr/sbin && \
	# check if executable works in this container
	/usr/local/bin/hippius --version

USER hippius

EXPOSE 30333 9933 9944 9615
VOLUME ["/data"]
ENTRYPOINT ["/usr/local/bin/hippius"]
