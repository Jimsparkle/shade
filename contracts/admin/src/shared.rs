use cosmwasm_std::Addr;

/// Maps user to permissions for which they have user.
pub const PERMISSIONS: Map<&Addr, Vec<String>> = Map::new("permissions");
/// List of all admins.
pub const ADMINS: Item<Vec<Addr>> = Item::new("admins");
/// Super user.
pub const SUPER: Item<Addr> = Item::new("super");
/// Whether or not this contract can be consumed.
pub const STATUS: Item<AdminAuthStatus> = Item::new("is_active");

pub fn validate_permissions(permissions: &[String]) -> AdminAuthResult<()> {
    for permission in permissions {
        is_valid_permission(permission.as_str())?;
    }
    Ok(())
}

pub fn is_valid_permission(permission: &str) -> AdminAuthResult<()> {
    if permission.len() <= 10 {
        return Err(AdminAuthError::InvalidPermissionFormat {
            permission: permission.to_string(),
        });
    }
    let valid_chars = permission.bytes().all(|byte| {
        (b'A'..=b'Z').contains(&byte) || (b'0'..=b'9').contains(&byte) || b'_'.eq(&byte)
    });
    if !valid_chars {
        return Err(AdminAuthError::InvalidPermissionFormat {
            permission: permission.to_string(),
        });
    }
    Ok(())
}