// E2E Test File for JavaScript/React Features
// This file tests JavaScript/JSX parsing capabilities:
// - ES6 class syntax with inheritance (extends)
// - JSX component invocations
// - Static methods and properties
// - Arrow functions
// - Callback references

/**
 * Base service class for testing inheritance
 */
class BaseService {
    constructor() {
        this.initialized = false;
    }

    /**
     * Initialize the service
     */
    init() {
        this.initialized = true;
    }

    /**
     * Get service status
     */
    getStatus() {
        return this.initialized;
    }
}

/**
 * Data service extending BaseService
 * Tests: class inheritance with extends
 */
class DataService extends BaseService {
    constructor() {
        super();
        this.data = [];
    }

    /**
     * Fetch data from API
     */
    async fetchData(url) {
        const response = await fetch(url);
        this.data = await response.json();
        return this.data;
    }

    /**
     * Static method for creating instances
     */
    static create() {
        return new DataService();
    }
}

/**
 * Utility class with static methods
 */
class Utils {
    /**
     * Format a date string
     */
    static formatDate(date) {
        return date.toISOString();
    }

    /**
     * Static constant
     */
    static get API_VERSION() {
        return 'v1';
    }
}

/**
 * React component using JSX
 * Tests: JSX component invocations
 */
class ChartToolbar extends React.Component {
    constructor(props) {
        super(props);
        this.state = { expanded: false };
        this.handleClick = this.handleClick.bind(this);
    }

    handleClick() {
        this.setState({ expanded: !this.state.expanded });
    }

    render() {
        return (
            <div id="chart-toolbar" className="toolbar toolbar-expanded">
                <Button onClick={this.handleClick} className="btn btn-primary">Toggle</Button>
                <Icons.Search size={24} />
                <Icons.Settings size={24} />
            </div>
        );
    }
}

/**
 * Dashboard component with nested JSX
 */
class Dashboard extends React.Component {
    constructor(props) {
        super(props);
        this.dataService = new DataService();
    }

    componentDidMount() {
        this.dataService.fetchData('/api/data');
    }

    render() {
        return (
            <Sheet.Container id="dashboard-container" className="dashboard">
                <Sheet.Header className="dashboard-header">
                    <ChartToolbar />
                </Sheet.Header>
                <Sheet.Content className="dashboard-content">
                    <DataGrid data={this.dataService.data} className="data-grid" />
                </Sheet.Content>
            </Sheet.Container>
        );
    }
}

/**
 * Functional component with arrow function
 */
const UserProfile = ({ user, onUpdate }) => {
    const handleSave = () => {
        onUpdate(user);
    };

    return (
        <div id="user-profile" className="profile-card shadow">
            <h1 className="profile-title">{user.name}</h1>
            <Button onClick={handleSave} className="btn btn-save">Save</Button>
        </div>
    );
};

/**
 * Hook-based component
 */
function useDataFetcher(url) {
    const [data, setData] = React.useState(null);

    React.useEffect(() => {
        const service = DataService.create();
        service.fetchData(url).then(setData);
    }, [url]);

    return data;
}

/**
 * Event handler registration (callback references)
 */
class EventManager {
    constructor() {
        this.handlers = [];
    }

    /**
     * Register event handler (tests callback reference extraction)
     */
    on(event, handler) {
        this.handlers.push({ event, handler });
    }

    /**
     * Usage example showing callback references
     */
    setupHandlers() {
        // Should capture these method references
        this.on('click', this.handleClick);
        this.on('submit', this.handleSubmit);
    }

    handleClick(e) {
        console.log('Click', e);
    }

    handleSubmit(e) {
        console.log('Submit', e);
    }
}

/**
 * Enum-like constant usage
 */
const Status = {
    IDLE: 'idle',
    LOADING: 'loading',
    SUCCESS: 'success',
    ERROR: 'error'
};

class ApiClient {
    constructor() {
        this.status = Status.IDLE;
    }

    async request(url) {
        this.status = Status.LOADING;
        try {
            const result = await fetch(url);
            this.status = Status.SUCCESS;
            return result;
        } catch (error) {
            this.status = Status.ERROR;
            throw error;
        }
    }
}

// Mock React namespace for standalone parsing
const React = {
    Component: class {},
    useState: () => [null, () => {}],
    useEffect: () => {}
};

// Mock UI components
const Button = ({ children, onClick }) => null;
const Icons = {
    Search: () => null,
    Settings: () => null
};
const Sheet = {
    Container: ({ children }) => null,
    Header: ({ children }) => null,
    Content: ({ children }) => null
};
const DataGrid = ({ data }) => null;

// Export for module testing
export { DataService, ChartToolbar, Dashboard, Utils, EventManager };
