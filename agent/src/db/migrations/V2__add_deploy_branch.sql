-- Add deploy_branch column to projects table
ALTER TABLE projects ADD COLUMN deploy_branch TEXT NOT NULL DEFAULT 'main';
