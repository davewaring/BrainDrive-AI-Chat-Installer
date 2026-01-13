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
    name: 'run_command',
    description: 'Execute a shell command on the user\'s system. Use this for installation tasks like cloning repos, installing dependencies, etc. Always explain what the command does before running it.',
    input_schema: {
      type: 'object',
      properties: {
        command: {
          type: 'string',
          description: 'The shell command to execute',
        },
      },
      required: ['command'],
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
