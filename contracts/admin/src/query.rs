use cosmwasm_std::Deps;

use crate::shared::{STATUS, SUPER, PERMISSIONS, is_valid_permission};

/// Checks if the user has the requested permission. Permissions are case sensitive.
pub fn query_validate_permission(
    deps: Deps,
    permission: String,
    user: String,
) -> AdminAuthResult<ValidateAdminPermissionResponse> {
    STATUS
        .load(deps.storage)?
        .not_shutdown()?
        .not_under_maintenance()?;
    is_valid_permission(permission.as_str())?;
    let valid_user = deps.api.addr_validate(user.as_str())?;
    let super_admin = SUPER.load(deps.storage)?;

    let has_permission: bool;

    // Super admin has all permissions. The permissions don't need to have been created and assigned to the super admin beforehand. We do this because we assume that the super admin is secure (like a multi-sig or the main governance contract) so it would be a hassle to whitelist every permission we want them to have.
    if valid_user == super_admin {
        has_permission = true;
    } else {
        let permissions = PERMISSIONS.may_load(deps.storage, &valid_user)?;
        match permissions {
            Some(permissions) => {
                if permissions.iter().any(|perm| permission.eq(perm)) {
                    has_permission = true;
                } else {
                    return Err(AdminAuthError::UnauthorizedAdmin {
                        user: valid_user,
                        permission,
                    });
                }
            }
            // If user has been registered, there should be an empty vector there.
            None => return Err(AdminAuthError::UnregisteredAdmin { user: valid_user }),
        }
    }
    Ok(ValidateAdminPermissionResponse { has_permission })
}

