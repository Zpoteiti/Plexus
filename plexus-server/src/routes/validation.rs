use crate::error::ApiError;

pub fn email(value: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() || !value.contains('@') {
        return Err(ApiError::invalid_args("email must be valid"));
    }
    Ok(())
}

pub fn name(value: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::invalid_args("name is required"));
    }
    Ok(())
}

pub fn password(value: &str) -> Result<(), ApiError> {
    if value.len() < 8 {
        return Err(ApiError::invalid_args(
            "password must be at least 8 characters",
        ));
    }
    Ok(())
}
