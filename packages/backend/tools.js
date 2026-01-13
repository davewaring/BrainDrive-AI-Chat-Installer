export const TOOLS = [
  {
    name: 'check_connection',
    description: 'Check if the bootstrapper app is connected. Use this to verify the user has downloaded and opened the bootstrapper before attempting any system operations.',
    input_schema: {
      type: 'object',
      properties: {},
      required: [],
    },
  },
  {
    name: 'detect_system',
    description: 'Detect the user\'s system information including OS, architecture, and installed software (conda, git, node, ollama). Use this early in the conversation to understand what needs to be installed.',
    input_schema: {
      type: 'object',
      properties: {},
      required: [],
    },
  },
  {
    name: 'install_conda_env',
    description: 'Create or update the audited BrainDrive conda environment. Provide the environment name and optionally a repo path/environment.yml override.',
    input_schema: {
      type: 'object',
      properties: {
        env_name: {
          type: 'string',
          description: 'The target conda environment name (letters, numbers, -, _)',
        },
        repo_path: {
          type: 'string',
          description: 'Absolute path to the BrainDrive repo (defaults to ~/BrainDrive)',
        },
        environment_file: {
          type: 'string',
          description: 'Relative path to the environment file inside the repo (defaults to environment.yml)',
        },
      },
      required: ['env_name'],
    },
  },
  {
    name: 'install_ollama',
    description: 'Install Ollama using the vetted installer script. Use only after the user confirms.',
    input_schema: {
      type: 'object',
      properties: {},
      required: [],
    },
  },
  {
    name: 'pull_ollama_model',
    description: 'Download an audited Ollama model (e.g., qwen2.5:1.5b).',
    input_schema: {
      type: 'object',
      properties: {
        model: {
          type: 'string',
          description: 'Model identifier to pull (letters, numbers, dots, :, /, -, _)',
        },
        registry: {
          type: 'string',
          description: 'Optional custom registry host to prefix before the model name',
        },
        force: {
          type: 'boolean',
          description: 'Force re-download even if cached locally',
        },
      },
      required: ['model'],
    },
  },
  {
    name: 'check_port_available',
    description: 'Check if a specific port is available for use. Use this before starting services to ensure ports are free.',
    input_schema: {
      type: 'object',
      properties: {
        port: {
          type: 'integer',
          description: 'The port number to check',
        },
      },
      required: ['port'],
    },
  },
  {
    name: 'clone_repo',
    description: 'Clone the BrainDrive repository from GitHub. This should be done early in the installation process. Uses shallow clone for faster download.',
    input_schema: {
      type: 'object',
      properties: {
        repo_url: {
          type: 'string',
          description: 'Repository URL (defaults to https://github.com/BrainDriveAI/BrainDrive.git)',
        },
        target_path: {
          type: 'string',
          description: 'Where to clone the repo (defaults to ~/BrainDrive)',
        },
      },
      required: [],
    },
  },
  {
    name: 'create_conda_env',
    description: 'Create a new conda environment with Python 3.11, Node.js, and git. Use this before installing dependencies.',
    input_schema: {
      type: 'object',
      properties: {
        env_name: {
          type: 'string',
          description: 'Name for the conda environment (defaults to BrainDriveDev)',
        },
      },
      required: [],
    },
  },
  {
    name: 'install_backend_deps',
    description: 'Install Python backend dependencies using pip in the conda environment. Run this after creating the conda env and cloning the repo.',
    input_schema: {
      type: 'object',
      properties: {
        env_name: {
          type: 'string',
          description: 'Conda environment name (defaults to BrainDriveDev)',
        },
        repo_path: {
          type: 'string',
          description: 'Path to the BrainDrive repo (defaults to ~/BrainDrive)',
        },
      },
      required: [],
    },
  },
  {
    name: 'install_frontend_deps',
    description: 'Install frontend npm dependencies. Run this after cloning the repo.',
    input_schema: {
      type: 'object',
      properties: {
        repo_path: {
          type: 'string',
          description: 'Path to the BrainDrive repo (defaults to ~/BrainDrive)',
        },
      },
      required: [],
    },
  },
  {
    name: 'setup_env_file',
    description: 'Set up the backend .env configuration file by copying .env-dev to .env. Run this before starting BrainDrive.',
    input_schema: {
      type: 'object',
      properties: {
        repo_path: {
          type: 'string',
          description: 'Path to the BrainDrive repo (defaults to ~/BrainDrive)',
        },
      },
      required: [],
    },
  },
  {
    name: 'start_braindrive',
    description: 'Start the BrainDrive backend and frontend services. Use this after installation is complete.',
    input_schema: {
      type: 'object',
      properties: {
        frontend_port: {
          type: 'integer',
          description: 'Port for the frontend (default: 5173)',
        },
        backend_port: {
          type: 'integer',
          description: 'Port for the backend (default: 8005)',
        },
      },
      required: [],
    },
  },
  {
    name: 'stop_braindrive',
    description: 'Stop the running BrainDrive services.',
    input_schema: {
      type: 'object',
      properties: {},
      required: [],
    },
  },
  {
    name: 'restart_braindrive',
    description: 'Restart the BrainDrive services. Useful after configuration changes.',
    input_schema: {
      type: 'object',
      properties: {},
      required: [],
    },
  },
];
