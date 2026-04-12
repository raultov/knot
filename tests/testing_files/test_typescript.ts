// E2E Test File for TypeScript/Angular Features
// This file tests all critical features added in v0.6.2:
// - Decorator references (@Component, @NgModule, @Injectable)
// - Type references in constructor parameters (dependency injection)
// - Interface implementation
// - Method signatures with type annotations
// - JSDoc comments

/**
 * Analytics service for tracking user behavior.
 * 
 * @remarks
 * This service is provided at root level and tracks page views.
 */
export class AnalyticsService {
    /**
     * Track a page view event
     */
    trackPageView(url: string): void {
        console.log(`Tracking page view: ${url}`);
    }

    /**
     * Track a custom event
     */
    trackEvent(eventName: string, data: EventData): void {
        console.log(`Event: ${eventName}`, data);
    }
}

/**
 * SEO service for managing meta tags
 */
export class SeoService {
    updateMeta(title: string, description: string): void {
        // Update meta tags
    }
}

/**
 * Event data interface
 */
export interface EventData {
    category: string;
    action: string;
    label?: string;
}

/**
 * Lifecycle hook interface
 */
export interface OnInit {
    ngOnInit(): void;
}

/**
 * Main application component with dependency injection.
 * Tests decorator and constructor type reference extraction.
 */
@Component({
    selector: 'ngx-app',
    template: '<router-outlet></router-outlet>',
})
export class AppComponent implements OnInit {
    private initialized = false;

    /**
     * Constructor with dependency injection.
     * Should capture AnalyticsService and SeoService as type references.
     */
    constructor(
        private analytics: AnalyticsService,
        private seo: SeoService
    ) {}

    /**
     * Component initialization
     */
    ngOnInit(): void {
        this.initialized = true;
        this.analytics.trackPageView('/home');
        this.seo.updateMeta('Home', 'Welcome to our app');
    }

    /**
     * Method with typed parameters and return type
     */
    processData(input: EventData): boolean {
        this.analytics.trackEvent('process', input);
        return true;
    }
}

/**
 * User component for testing multiple declarations
 */
@Component({
    selector: 'ngx-user',
    template: '<div>User Profile</div>',
})
export class UserComponent {
    userName: string;

    constructor(private analytics: AnalyticsService) {
        this.userName = 'Guest';
    }
}

/**
 * Application module with decorator references.
 * Should capture AppComponent and UserComponent from declarations.
 */
@NgModule({
    declarations: [
        AppComponent,
        UserComponent
    ],
    providers: [
        AnalyticsService,
        SeoService
    ],
    bootstrap: [AppComponent]
})
export class AppModule { }

/**
 * Injectable service decorator test
 */
@Injectable({
    providedIn: 'root'
})
export class ConfigService {
    private config: Record<string, any> = {};

    getConfig(key: string): any {
        return this.config[key];
    }
}

// Decorator placeholder (would be imported from @angular/core in real code)
function Component(config: any): ClassDecorator {
    return (target: any) => target;
}

function NgModule(config: any): ClassDecorator {
    return (target: any) => target;
}

function Injectable(config: any): ClassDecorator {
    return (target: any) => target;
}
