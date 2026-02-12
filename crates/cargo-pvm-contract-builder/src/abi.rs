use anyhow::{Context, Result};

/// Extract ABI JSON from the `__PVM_ABI` symbol in an ELF binary.
///
/// The contract macro embeds ABI JSON via a `#[link_section = ".rodata.pvm_abi"]` static,
/// but the linker merges that section into `.rodata`. We look up the `__PVM_ABI` symbol
/// by name instead, which works regardless of section merging.
pub fn extract_abi_from_elf(elf_bytes: &[u8]) -> Result<Option<String>> {
    use object::{Object, ObjectSection, ObjectSymbol};

    let obj = object::File::parse(elf_bytes).context("Failed to parse ELF binary")?;

    let symbol = obj.symbols().find(|s| s.name() == Ok("__PVM_ABI"));

    let sym = match symbol {
        Some(s) => s,
        None => return Ok(None),
    };

    let size = sym.size() as usize;
    if size == 0 {
        return Ok(None);
    }

    let section_index = sym
        .section_index()
        .context("__PVM_ABI symbol has no section")?;
    let section = obj
        .section_by_index(section_index)
        .context("Failed to find section for __PVM_ABI")?;
    let sec_data = section.data().context("Failed to read section data")?;

    let offset = (sym.address() - section.address()) as usize;
    let abi_data = sec_data
        .get(offset..offset + size)
        .context("__PVM_ABI symbol data out of section bounds")?;

    let json =
        std::str::from_utf8(abi_data).context("__PVM_ABI symbol contains invalid UTF-8")?;

    Ok(Some(json.to_string()))
}

/// Extract CDM metadata JSON from the `__PVM_CDM` symbol in an ELF binary.
///
/// The contract macro embeds CDM metadata (package name) via a
/// `#[link_section = ".rodata.pvm_cdm"]` static when the contract has
/// a `cdm = "..."` attribute.
pub fn extract_cdm_from_elf(elf_bytes: &[u8]) -> Result<Option<String>> {
    use object::{Object, ObjectSection, ObjectSymbol};

    let obj = object::File::parse(elf_bytes).context("Failed to parse ELF binary")?;

    let symbol = obj.symbols().find(|s| s.name() == Ok("__PVM_CDM"));

    let sym = match symbol {
        Some(s) => s,
        None => return Ok(None),
    };

    let size = sym.size() as usize;
    if size == 0 {
        return Ok(None);
    }

    let section_index = sym
        .section_index()
        .context("__PVM_CDM symbol has no section")?;
    let section = obj
        .section_by_index(section_index)
        .context("Failed to find section for __PVM_CDM")?;
    let sec_data = section.data().context("Failed to read section data")?;

    let offset = (sym.address() - section.address()) as usize;
    let cdm_data = sec_data
        .get(offset..offset + size)
        .context("__PVM_CDM symbol data out of section bounds")?;

    let json =
        std::str::from_utf8(cdm_data).context("__PVM_CDM symbol contains invalid UTF-8")?;

    Ok(Some(json.to_string()))
}
