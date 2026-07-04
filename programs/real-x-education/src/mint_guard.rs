use anchor_lang::prelude::*;
use anchor_spl::token::spl_token;
use anchor_spl::token_2022::spl_token_2022::{
    extension::{BaseStateWithExtensions, ExtensionType, StateWithExtensions},
    state::Mint as MintState,
};

use crate::error::EducationError;

/// Reject mints carrying an extension that breaks the one-to-one escrow
/// accounting or lets a third party move, freeze, or hide escrowed funds.
/// Classic SPL mints have no extensions, so they skip the check.
pub fn require_supported_mint(mint: &AccountInfo) -> Result<()> {
    if *mint.owner == spl_token::ID {
        return Ok(());
    }
    let data = mint.try_borrow_data()?;
    let state = StateWithExtensions::<MintState>::unpack(&data)?;
    for extension in state.get_extension_types()? {
        match extension {
            ExtensionType::TransferFeeConfig
            | ExtensionType::MintCloseAuthority
            | ExtensionType::DefaultAccountState
            | ExtensionType::NonTransferable
            | ExtensionType::PermanentDelegate
            | ExtensionType::TransferHook
            | ExtensionType::Pausable
            | ExtensionType::ConfidentialTransferMint
            | ExtensionType::ConfidentialMintBurn => {
                return err!(EducationError::UnsupportedMintExtension);
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t22_mint_data(extension_type: u16, len: u16) -> Vec<u8> {
        // Base mint (82 bytes) padded to the account type offset (165), the
        // mint tag, then a single TLV entry header plus a zeroed payload.
        let mut data = vec![0u8; 166 + 4 + len as usize];
        data[45] = 1; // is_initialized
        data[165] = 1; // account type: mint
        data[166..168].copy_from_slice(&extension_type.to_le_bytes());
        data[168..170].copy_from_slice(&len.to_le_bytes());
        data
    }

    fn check(owner: Pubkey, mut data: Vec<u8>) -> Result<()> {
        let key = Pubkey::new_unique();
        let mut lamports = 0u64;
        let info = AccountInfo::new(&key, false, false, &mut lamports, &mut data, &owner, false);
        require_supported_mint(&info)
    }

    #[test]
    fn classic_mint_passes() {
        assert!(check(spl_token::ID, vec![0u8; 82]).is_ok());
    }

    #[test]
    fn fee_bearing_mint_rejected() {
        // Extension type 1 is the transfer fee config.
        assert!(check(anchor_spl::token_2022::ID, t22_mint_data(1, 108)).is_err());
    }

    #[test]
    fn permanent_delegate_rejected() {
        // Extension type 12 is the permanent delegate.
        assert!(check(anchor_spl::token_2022::ID, t22_mint_data(12, 32)).is_err());
    }

    #[test]
    fn pausable_mint_rejected() {
        // Extension type 26 is the pausable config.
        assert!(check(anchor_spl::token_2022::ID, t22_mint_data(26, 33)).is_err());
    }

    #[test]
    fn metadata_pointer_allowed() {
        // Extension type 18 is the metadata pointer, which is harmless here.
        assert!(check(anchor_spl::token_2022::ID, t22_mint_data(18, 64)).is_ok());
    }
}
