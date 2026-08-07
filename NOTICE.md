# Notices for SDV Fleet Management

This content is produced and maintained by Lunatech as a demonstrator built on
the [Eclipse SDV Fleet Management blueprint](https://github.com/eclipse-sdv-blueprints/fleet-management),
with the intent of contributing parts of it upstream.

* Blueprint home: https://projects.eclipse.org/projects/automotive.sdv-blueprints

## Trademarks

Eclipse SDV Blueprints is a trademark of the Eclipse Foundation. Eclipse, Eclipse
Kuksa, Eclipse hawkBit, Eclipse Zenoh, Eclipse uProtocol, Eclipse Mosquitto and
the Eclipse Logo are trademarks or registered trademarks of the Eclipse
Foundation.

## Copyright

All content is the property of the respective authors or their employers. For
more information regarding authorship of content, please consult the listed
source code repository logs.

## Declared Project Licenses

This program and the accompanying materials are made available under the terms
of the Apache License, Version 2.0 which is available at
http://www.apache.org/licenses/LICENSE-2.0

SPDX-License-Identifier: Apache-2.0

This project was previously distributed under the Eclipse Public License 2.0.
It was relicensed to Apache-2.0 so that components can be contributed to the
Eclipse SDV Fleet Management blueprint, which is Apache-2.0. All contributors at
the time of the change were employees of Lunatech.

## Source Code

The project maintains the following source code repositories:

* https://github.com/lunatech-labs/sdv-fleet-management

## Third-Party Content

The following files are derived from the Eclipse SDV Fleet Management blueprint
and remain under its Apache-2.0 license:

* `csv-provider/signalsFmsRecording.csv`
* `csv-provider/signalsCovesaCvRecording.csv`
* `influxdb/` (InfluxDB configuration and init script)
* `config/zenoh/` (Zenoh router and client configuration)
* `spec/overlay/vss.json` (VSS model, extended by `spec/overlay/ota.vspec`)

`ota-agent/proto/kuksa/val/v1/` contains protobuf definitions from the
[Eclipse Kuksa Databroker](https://github.com/eclipse-kuksa/kuksa-databroker),
Apache-2.0.

The stack additionally runs unmodified container images published by the Eclipse
SDV Blueprints project (`fms-forwarder`, `fms-consumer`), the Eclipse Kuksa
project (`kuksa-databroker`, `csv-provider`), Eclipse hawkBit and Eclipse Zenoh.

## Cryptography

Content may contain encryption software. The country in which you are currently
may have restrictions on the import, possession, and use, and/or re-export to
another country, of encryption software. BEFORE using any encryption software,
please check the country's laws, regulations and policies concerning the import,
possession, or use, and re-export of encryption software, to see if this is
permitted.
