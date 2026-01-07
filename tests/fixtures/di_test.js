// di_test.js - DI check test cases
angular.module('diTestApp', [])

// Service definition
.service('MyService', function() {
    this.doSomething = function() {
        return 'done';
    };
})

// Controller with MyService DI - should allow jump
.controller('WithDIController', ['$scope', 'MyService', function($scope, MyService) {
    // This should work - MyService is DI'd
    MyService.doSomething();
}])

// Controller without MyService DI - should NOT allow jump
.controller('WithoutDIController', ['$scope', function($scope) {
    // This should NOT work - MyService is NOT DI'd
    MyService.doSomething();
}])

// $inject pattern test
function InjectController($scope, MyService) {
    // This should work - MyService is DI'd via $inject
    MyService.doSomething();
}
InjectController.$inject = ['$scope', 'MyService'];

// $inject pattern without MyService
function NoInjectController($scope) {
    // This should NOT work - MyService is NOT DI'd
    MyService.doSomething();
}
NoInjectController.$inject = ['$scope'];
