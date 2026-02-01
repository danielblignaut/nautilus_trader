// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
//  https://nautechsystems.io
//
//  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
//  You may not use this file except in compliance with the License.
//  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
// -------------------------------------------------------------------------------------------------

//! Provides a factory for creating different account types with registration support.
//!
//! The factory supports:
//! - Registering calculated accounts (accounts with dynamically computed balances from fills)
//! - Registering cash borrowing (allowing negative balances for cash accounts)

use std::sync::{Mutex, OnceLock};

use ahash::AHashSet;

/// Global registry of issuers with calculated account states.
/// Accounts registered here have their balances computed from order fills rather than
/// relying on reported balances from the venue.
fn calculated_accounts() -> &'static Mutex<AHashSet<String>> {
    static INSTANCE: OnceLock<Mutex<AHashSet<String>>> = OnceLock::new();
    INSTANCE.get_or_init(|| Mutex::new(AHashSet::new()))
}

/// Global registry of issuers that allow cash borrowing (negative balances).
fn cash_borrowing() -> &'static Mutex<AHashSet<String>> {
    static INSTANCE: OnceLock<Mutex<AHashSet<String>>> = OnceLock::new();
    INSTANCE.get_or_init(|| Mutex::new(AHashSet::new()))
}

/// Account factory for creating accounts with proper configuration.
#[derive(Debug)]
pub struct AccountFactory;

impl AccountFactory {
    /// Register an issuer for calculated account state.
    ///
    /// Calculated accounts have their balances computed dynamically from order fills
    /// rather than using reported balances from the venue. This is essential for
    /// backtesting and simulation environments.
    ///
    /// # Arguments
    /// * `issuer` - The issuer identifier (typically the venue/exchange ID)
    ///
    /// # Errors
    /// Returns an error if the issuer is already registered.
    pub fn register_calculated_account(issuer: &str) -> anyhow::Result<()> {
        let mut accounts = calculated_accounts().lock().unwrap();
        if accounts.contains(issuer) {
            anyhow::bail!(
                "Issuer '{}' is already registered as a calculated account",
                issuer
            );
        }
        accounts.insert(issuer.to_string());
        log::debug!("Registered calculated account for issuer: {}", issuer);
        Ok(())
    }

    /// Register an issuer for cash borrowing (negative balances).
    ///
    /// Cash accounts normally cannot have negative balances. This registration
    /// allows specific issuers to support borrowing (negative balances).
    ///
    /// # Arguments
    /// * `issuer` - The issuer identifier (typically the venue/exchange ID)
    ///
    /// # Errors
    /// Returns an error if the issuer is already registered.
    pub fn register_cash_borrowing(issuer: &str) -> anyhow::Result<()> {
        let mut borrowing = cash_borrowing().lock().unwrap();
        if borrowing.contains(issuer) {
            anyhow::bail!(
                "Issuer '{}' is already registered for cash borrowing",
                issuer
            );
        }
        borrowing.insert(issuer.to_string());
        log::debug!("Registered cash borrowing for issuer: {}", issuer);
        Ok(())
    }

    /// Deregister an issuer from cash borrowing.
    ///
    /// Primarily intended for test cleanup to prevent global state leakage.
    ///
    /// # Arguments
    /// * `issuer` - The issuer identifier to deregister
    pub fn deregister_cash_borrowing(issuer: &str) {
        let mut borrowing = cash_borrowing().lock().unwrap();
        borrowing.remove(issuer);
        log::debug!("Deregistered cash borrowing for issuer: {}", issuer);
    }

    /// Check if an issuer is registered for calculated account state.
    ///
    /// # Arguments
    /// * `issuer` - The issuer identifier to check
    ///
    /// # Returns
    /// `true` if the issuer is registered for calculated accounts
    pub fn is_calculated_account(issuer: &str) -> bool {
        let accounts = calculated_accounts().lock().unwrap();
        accounts.contains(issuer)
    }

    /// Check if an issuer is registered for cash borrowing.
    ///
    /// # Arguments
    /// * `issuer` - The issuer identifier to check
    ///
    /// # Returns
    /// `true` if the issuer allows cash borrowing
    pub fn is_cash_borrowing(issuer: &str) -> bool {
        let borrowing = cash_borrowing().lock().unwrap();
        borrowing.contains(issuer)
    }

    /// Clear all registrations (primarily for testing).
    pub fn clear_registrations() {
        let mut accounts = calculated_accounts().lock().unwrap();
        accounts.clear();
        let mut borrowing = cash_borrowing().lock().unwrap();
        borrowing.clear();
        log::debug!("Cleared all account factory registrations");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() {
        AccountFactory::clear_registrations();
    }

    #[test]
    fn test_register_calculated_account() {
        setup();
        assert!(!AccountFactory::is_calculated_account("TEST"));

        AccountFactory::register_calculated_account("TEST").unwrap();
        assert!(AccountFactory::is_calculated_account("TEST"));
    }

    #[test]
    fn test_register_calculated_account_duplicate_fails() {
        setup();
        AccountFactory::register_calculated_account("TEST").unwrap();
        assert!(AccountFactory::register_calculated_account("TEST").is_err());
    }

    #[test]
    fn test_register_cash_borrowing() {
        setup();
        assert!(!AccountFactory::is_cash_borrowing("TEST"));

        AccountFactory::register_cash_borrowing("TEST").unwrap();
        assert!(AccountFactory::is_cash_borrowing("TEST"));
    }

    #[test]
    fn test_deregister_cash_borrowing() {
        setup();
        AccountFactory::register_cash_borrowing("TEST").unwrap();
        assert!(AccountFactory::is_cash_borrowing("TEST"));

        AccountFactory::deregister_cash_borrowing("TEST");
        assert!(!AccountFactory::is_cash_borrowing("TEST"));
    }

    #[test]
    fn test_clear_registrations() {
        setup();
        AccountFactory::register_calculated_account("TEST").unwrap();
        AccountFactory::register_cash_borrowing("TEST").unwrap();

        AccountFactory::clear_registrations();

        assert!(!AccountFactory::is_calculated_account("TEST"));
        assert!(!AccountFactory::is_cash_borrowing("TEST"));
    }
}
