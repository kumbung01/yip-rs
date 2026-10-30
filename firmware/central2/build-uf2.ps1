$ErrorActionPreference = 'Stop'

Push-Location $PSScriptRoot
try {
    $firmwareTarget = Join-Path $PSScriptRoot 'target'
    $firmwareElf = Join-Path $firmwareTarget 'thumbv7em-none-eabihf/release/yip-central2'
    $firmwareBin = "$firmwareElf.bin"
    $firmwareUf2 = "$firmwareElf.uf2"

    cargo build --release --locked --target thumbv7em-none-eabihf --target-dir $firmwareTarget
    if ($LASTEXITCODE -ne 0) { throw 'Firmware build failed.' }

    # Verify the ELF link address before converting to a raw binary.
    $sections = & rust-objdump -h $firmwareElf
    if ($LASTEXITCODE -ne 0) { throw 'ELF inspection failed. Install cargo-binutils and llvm-tools.' }
    if (-not ($sections -match '\.vector_table\s+[0-9a-fA-F]+\s+0*1000\s+')) {
        throw 'Expected the vector table at 0x1000 for a bootloader without SoftDevice.'
    }

    rust-objcopy -O binary $firmwareElf $firmwareBin
    if ($LASTEXITCODE -ne 0) { throw 'Binary conversion failed.' }

    python (Join-Path $PSScriptRoot 'make_uf2.py') $firmwareBin $firmwareUf2
    if ($LASTEXITCODE -ne 0) { throw 'UF2 conversion failed.' }

    Write-Host "Copy this file to the NICENANO bootloader drive:"
    Write-Host $firmwareUf2
}
finally {
    Pop-Location
}
