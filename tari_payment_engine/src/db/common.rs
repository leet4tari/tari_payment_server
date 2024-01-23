use crate::db_types::{NewOrder, NewPayment, Order, OrderId, TransferStatus, UserAccount};
use tari_common_types::tari_address::TariAddress;

pub enum InsertOrderResult {
    Inserted(i64),
    AlreadyExists(i64),
}

pub enum InsertPaymentResult {
    Inserted(String),
    AlreadyExists(String),
}

#[allow(async_fn_in_trait)]
pub trait PaymentGatewayDatabase: Clone {
    type Error: std::error::Error;
    /// Fetches the user account for the given customer_id and/or public key. If both customer_id and public_key are
    /// provided, the resulting account id must match, otherwise an error is returned.
    ///
    /// If the account does not exist, one is created and the given customer id and/or public key is linked to the
    /// account.
    async fn fetch_or_create_account(
        &self,
        cust_id: Option<NewOrder>,
        pubkey: Option<NewPayment>,
    ) -> Result<i64, Self::Error>;

    /// Takes a new order, and in a single atomic transaction,
    /// * calls `save_new_order` to store the order in the database. If the order already exists,
    ///   nothing further is done.
    /// * creates a new account for the customer if one does not already exist
    /// * Updates the total orders value for the account
    ///
    /// Returns the account id for the customer.
    async fn process_new_order_for_customer(&self, order: NewOrder) -> Result<i64, Self::Error>;

    /// Takes a new payment, and in a single atomic transaction,
    /// * calls `save_payment` to store the payment in the database. If the payment already exists,
    ///   nothing further is done.
    /// * The payment is marked as `Unconfirmed`
    /// * creates a new account for the public key if one does not already exist
    /// Returns the account id for the public key.
    async fn process_new_payment_for_pubkey(&self, payment: NewPayment)
        -> Result<i64, Self::Error>;

    /// Checks whether any orders associated with the given account id can be fulfilled.
    /// If no orders can be fulfilled, an empty vector is returned.
    async fn fetch_payable_orders(&self, account_id: i64) -> Result<Vec<Order>, Self::Error>;

    /// Tries to fulfil the list of arders given from the given account.
    ///
    /// Any order that has enough credit in the account
    /// * Will be marked as Paid
    /// * Returned in the result vector.
    async fn try_pay_orders(
        &self,
        account_id: i64,
        orders: &[Order],
    ) -> Result<Vec<Order>, Self::Error>;

    /// Updates the payment status for the given transaction id. This is typically called to transition a payment from
    /// `Unconfirmed` to `Confirmed` or `Cancelled`.
    ///
    /// If the transaction was not "Received", an error is returned.
    /// If the status is unchanged, then nothing is changed, and `None` is returned.
    ///
    /// If the status is changed, the account id corresponding to the transaction is returned.
    async fn update_payment_status(
        &self,
        tx_id: &str,
        status: TransferStatus,
    ) -> Result<Option<i64>, Self::Error>;

    /// Closes the database connection.
    async fn close(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[allow(async_fn_in_trait)]
pub trait AccountManagement {
    type Error: std::error::Error;
    /// Fetches the user account associated with the given account id. If no account exists, `None` is returned.
    async fn fetch_user_account(&self, account_id: i64)
        -> Result<Option<UserAccount>, Self::Error>;

    /// Fetches the user account associated with the given order id. If no account exists, `None` is returned.
    async fn fetch_user_account_for_order(
        &self,
        order_id: &OrderId,
    ) -> Result<Option<UserAccount>, Self::Error>;

    async fn fetch_user_account_for_customer_id(
        &self,
        customer_id: &str,
    ) -> Result<Option<UserAccount>, Self::Error>;
    async fn fetch_user_account_for_pubkey(
        &self,
        pubkey: &TariAddress,
    ) -> Result<Option<UserAccount>, Self::Error>;
}

#[macro_export]
macro_rules! op {
    (binary $for_struct:ident, $impl_trait:ident, $impl_fn:ident) => {
        impl $impl_trait for $for_struct {
            type Output = Self;
            fn $impl_fn(self, rhs: Self) -> Self::Output {
                Self(self.0.$impl_fn(rhs.0))
            }
        }
    };

    (inplace $for_struct:ident, $impl_trait:ident, $impl_fn:ident) => {
        impl $impl_trait for $for_struct {
            fn $impl_fn(&mut self, rhs: Self) {
                self.0.$impl_fn(rhs.0)
            }
        }
    };

    (unary $for_struct:ident, $impl_trait:ident, $impl_fn:ident) => {
        impl $impl_trait for $for_struct {
            type Output = Self;
            fn $impl_fn(self) -> Self::Output {
                Self(self.0.$impl_fn())
            }
        }
    };
}
