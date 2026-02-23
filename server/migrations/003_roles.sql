-- Add role column: 'admin' or 'user'
ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'user';

-- First user in the system should be admin (set by setup handler)
