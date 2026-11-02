# nice!nano / nRF52840 DK 슬롯 테스트

DK는 2개 pipe를 poll하고 각 nice!nano는 부팅 후 자동으로 연결합니다. 연결되면 송신 큐에 8바이트 패킷을 계속 공급합니다. 앞 4바이트는 순번이고 뒤 4바이트는 peripheral에서 측정한 생성 시각입니다. 기본 슬롯은 500 µs이므로 정상적인 두 링크의 pipe별 poll 간격은 1 ms입니다.

## 업로드와 실행

1. 저장소 루트에서 다음 명령으로 nice!nano 이미지 두 개를 만드세요.

   ```powershell
   .\firmware\nicenano-peripheral\build-uf2.ps1 -Pipe 0
   .\firmware\nicenano-peripheral\build-uf2.ps1 -Pipe 1
   ```

2. nice!nano를 더블 리셋하고 각 보드의 `INFO_UF2.TXT`에서 `SoftDevice: not found`를 확인하세요. 이 빌드는 저장소의 nice!nano 0.6.0 부트로더용 메모리 배치(앱 시작 주소 0x1000)를 사용합니다. 첫 보드에는 `target/pipe-0.uf2`, 두 번째에는 `target/pipe-1.uf2`를 해당 NICENANO 드라이브로 복사하세요.
3. DK에서 RTT를 시작하세요. 저장소 runner에 등록한 프로브가 다르면 `probe-rs list`로 DK 식별자를 확인하고 runner를 맞추세요.

   ```powershell
   cd C:\git\rust\yip-rs\firmware\nrf52-central
   cargo run --release --locked --features slot-test --bin slot-test
   ```
