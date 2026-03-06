use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use crate::error::{Result, ServerError};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    pub iat: usize,
    pub permissions: Vec<String>,
}

impl Claims {
    pub fn new(sub: String, permissions: Vec<String>, expires_in: usize) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize;
        
        Claims {
            sub,
            exp: now + expires_in,
            iat: now,
            permissions,
        }
    }
    
    pub fn has_permission(&self, permission: &str) -> bool {
        self.permissions.iter().any(|p| p == permission)
    }
}

pub fn generate_jwt(
    sub: String,
    permissions: Vec<String>,
    secret: &str,
    expires_in: usize,
) -> Result<String> {
    let claims = Claims::new(sub, permissions, expires_in);
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    );
    
    token.map_err(|e| ServerError::AuthError(e.to_string()))
}

pub fn validate_jwt(token: &str, secret: &str) -> Result<Claims> {
    let mut validation = Validation::default();
    validation.validate_exp = true; // Enable expiration validation
    
    let decoded = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    );
    
    decoded
        .map(|token_data| token_data.claims)
        .map_err(|e| ServerError::AuthError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_jwt_generation_and_validation() {
        let secret = "test-secret";
        let sub = "user123".to_string();
        let permissions = vec!["read".to_string(), "write".to_string()];
        
        let token = generate_jwt(sub.clone(), permissions.clone(), secret, 3600).unwrap();
        let claims = validate_jwt(&token, secret).unwrap();
        
        assert_eq!(claims.sub, sub);
        assert!(claims.has_permission("read"));
        assert!(claims.has_permission("write"));
        assert!(!claims.has_permission("delete"));
    }
    
    #[test]
    fn test_jwt_with_invalid_secret() {
        let token = generate_jwt("user123".to_string(), vec![], "secret1", 3600).unwrap();
        let result = validate_jwt(&token, "wrong-secret");
        assert!(result.is_err());
    }
    
    #[test]
    fn test_jwt_expiration() {
        let secret = "test-secret";
        let sub = "user123".to_string();
        let permissions = vec!["read".to_string()];
        
        // Generate token that expires in the past
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize;
        
        let claims = Claims {
            sub: sub.clone(),
            exp: now - 1000, // Expired 1000 seconds ago
            iat: now,
            permissions: permissions.clone(),
        };
        
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        ).unwrap();
        
        let result = validate_jwt(&token, secret);
        assert!(result.is_err());
    }
}